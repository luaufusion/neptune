use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{parse_macro_input, ItemFn, FnArg, Pat, Ident, Visibility, Token, LitBool, LitStr, Expr};
use syn::parse::{Parse, ParseStream};

struct GlobalsDef {
    vis: Visibility,
    name: Ident,
    bindings: Vec<(String, Ident)>,
    bootstrap: bool,
    js_files: Vec<(LitStr, Expr)>,
}

impl Parse for GlobalsDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let vis: Visibility = input.parse()?;
        input.parse::<Token![struct]>()?;
        let name: Ident = input.parse()?;
        
        let content;
        syn::braced!(content in input);
        
        let mut bindings = Vec::new();
        let mut js_files = Vec::new();
        let mut bootstrap = false;

        while !content.is_empty() {
            let key: Ident = content.parse()?;
            if key == "bootstrap" {
                content.parse::<Token![:]>()?;
                let val: LitBool = content.parse()?;
                bootstrap = val.value;
            } else if key == "js_files" {
                content.parse::<Token![:]>()?;
                let files_content;
                syn::braced!(files_content in content);
                while !files_content.is_empty() {
                    let file_name: LitStr = files_content.parse()?;
                    files_content.parse::<Token![=>]>()?;
                    let expr: Expr = files_content.parse()?;
                    js_files.push((file_name, expr));
                    if !files_content.is_empty() {
                        files_content.parse::<Token![,]>()?;
                    }
                }
            } else {
                content.parse::<Token![:]>()?;
                let func: Ident = content.parse()?;
                bindings.push((key.to_string(), func));
            }
            
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }
        
        Ok(GlobalsDef { vis, name, bindings, bootstrap, js_files })
    }
}

#[proc_macro]
pub fn define_globals(item: TokenStream) -> TokenStream {
    let GlobalsDef { vis, name, bindings, bootstrap, js_files } = parse_macro_input!(item as GlobalsDef);

    let mut registers = Vec::new();
    let mut references = Vec::new();

    let target_obj = if bootstrap {
        quote! { bobj }
    } else {
        quote! { global }
    };

    let bootstrap_init = if bootstrap {
        quote! { let bobj = Self::get_bootstrap(scope, global); }
    } else {
        quote! {}
    };

    for (key, func) in bindings {
        registers.push(quote! {
            Self::add(scope, #target_obj, #key, #func);
        });
        references.push(quote! {
            (#key, #func.map_fn_to())
        });
    }

    let js_files_impl = if js_files.is_empty() {
        quote! {}
    } else {
        let file_pairs = js_files.into_iter().map(|(name, expr)| {
            quote! {
                (std::borrow::Cow::Borrowed(#name), std::borrow::Cow::Borrowed(#expr))
            }
        });
        quote! {
            fn js_files() -> Vec<(std::borrow::Cow<'static, str>, std::borrow::Cow<'static, str>)> {
                vec![
                    #(#file_pairs),*
                ]
            }
        }
    };

    let expanded = quote! {
        #vis struct #name;
        impl crate::extension::Globals for #name {
            fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
                #bootstrap_init
                #(#registers)*
            }

            fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
                use v8::MapFnTo;
                vec![#(#references),*]
            }

            #js_files_impl
        }
    };

    expanded.into()
}

#[proc_macro_attribute]
pub fn op(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut is_async = false;
    let mut is_nonreentrant = false;
    let mut is_raw = false;

    // Parse the attributes as a comma-separated list of identifiers (e.g. `#[op(async, raw)]`)
    let attrs = parse_macro_input!(attr with Punctuated::<Ident, Comma>::parse_terminated);

    for ident in attrs {
        if ident == "async" {
            is_async = true;
        } else if ident == "nonreentrant" {
            is_nonreentrant = true;
        } else if ident == "raw" {
            is_raw = true;
        } else {
            return syn::Error::new_spanned(ident, "Unknown op attribute. Expected 'async', 'nonreentrant', or 'raw'")
                .to_compile_error()
                .into();
        }
    }

    let input = parse_macro_input!(item as ItemFn);
    let sig = &input.sig;
    let vis = &input.vis;
    let name = &sig.ident;
    let name_impl = Ident::new(&format!("{}_impl", name), name.span());

    let mut impl_sig = sig.clone();
    impl_sig.ident = name_impl.clone();
    
    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();

    let inputs = &sig.inputs;
    if inputs.is_empty() {
        return syn::Error::new_spanned(sig, "Op functions must have at least one argument (scope)")
        .to_compile_error()
        .into();    
    }

    // Skip the first argument (scope)
    for arg in inputs.iter().skip(1) {
        if let FnArg::Typed(pat_type) = arg {
            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                arg_names.push(pat_ident.ident.clone());
                arg_types.push(pat_type.ty.clone());
            } else {
                return syn::Error::new_spanned(pat_type, "Complex patterns in op arguments are not supported")
                .to_compile_error()
                .into();            }
        } else {
            return syn::Error::new_spanned(arg, "self argument is not supported in op functions")
            .to_compile_error()
            .into();        
        }
    }
    
    let wrap_ident = if is_async {
        quote! { ::neptune_runtime::extension::wrap_async }
    } else if is_nonreentrant {
        quote! { ::neptune_runtime::extension::wrap_nonreentrant }
    } else if is_raw {
        quote! { ::neptune_runtime::extension::wrap_raw }
    } else {
        quote! { ::neptune_runtime::extension::wrap }
    };

    let wrapper_func = if is_raw {
        quote! {
            #vis fn #name<'s>(
                scope: &mut v8::PinScope<'s, '_>,
                args: v8::FunctionCallbackArguments<'s>,
                mut retval: v8::ReturnValue,
            ) {
                #wrap_ident(scope, args, retval, |scope, args, retval| {
                    #name_impl(scope, args, retval)
                });
            }
        }
    } else {
        quote! {
            #vis fn #name<'s>(
                scope: &mut v8::PinScope<'s, '_>,
                args: v8::FunctionCallbackArguments<'s>,
                mut retval: v8::ReturnValue,
            ) {
                #wrap_ident(scope, args, retval, |scope, (#(#arg_names,)*): (#(#arg_types,)*)| {
                    #name_impl(scope, #(#arg_names,)*)
                });
            }
        }
    };

    let body = input.block;

    // Use the original function logic, but split it
    let expanded = quote! {
        #wrapper_func

        #[inline(always)]
        #vis #impl_sig {
            #body
        }
    };

    expanded.into()
}

#[proc_macro_derive(FromV8)]
pub fn derive_from_v8(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;

    let syn::Data::Struct(data_struct) = &input.data else {
        return syn::Error::new_spanned(name, "FromV8 can only be derived for structs")
            .to_compile_error()
            .into();
    };

    let fields = match &data_struct.fields {
        syn::Fields::Named(fields) => &fields.named,
        _ => {
            return syn::Error::new_spanned(name, "FromV8 can only be derived for structs with named fields")
                .to_compile_error()
                .into();
        }
    };

    let mut field_reads = Vec::new();
    let mut field_names = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();

        field_reads.push(quote! {
            let key = v8::String::new(scope, #field_name_str).unwrap();
            let value = obj.get(scope, key.into()).unwrap_or_else(|| v8::undefined(scope).into());
            let #field_name = ::neptune_runtime::extension::FromV8::from_v8(scope, value)?;
        });
        field_names.push(field_name);
    }

    let generics = &input.generics;
    let mut impl_generics_with_s = generics.clone();
    if !impl_generics_with_s.params.iter().any(|p| matches!(p, syn::GenericParam::Lifetime(l) if l.lifetime.ident == "s")) {
        impl_generics_with_s.params.push(syn::parse_quote!('s));
    }
    let (impl_generics, _, _) = impl_generics_with_s.split_for_impl();
    let (_, ty_generics, where_clause) = generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics ::neptune_runtime::extension::FromV8<'s> for #name #ty_generics #where_clause {
            fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, ::neptune_runtime::extension::NeptuneError> {
                if !value.is_object() {
                    return Err(::neptune_runtime::extension::NeptuneError::StaticTypeError("Expected an object"));
                }
                let obj: v8::Local<v8::Object> = value.try_into().map_err(|_| ::neptune_runtime::extension::NeptuneError::StaticTypeError("Expected an object"))?;
                
                #(#field_reads)*

                Ok(Self {
                    #(#field_names),*
                })
            }
        }
    };

    expanded.into()
}

#[proc_macro_derive(IntoV8)]
pub fn derive_into_v8(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;

    let syn::Data::Struct(data_struct) = &input.data else {
        return syn::Error::new_spanned(name, "IntoV8 can only be derived for structs")
            .to_compile_error()
            .into();
    };

    let fields = match &data_struct.fields {
        syn::Fields::Named(fields) => &fields.named,
        _ => {
            return syn::Error::new_spanned(name, "IntoV8 can only be derived for structs with named fields")
                .to_compile_error()
                .into();
        }
    };

    let mut field_writes = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();

        field_writes.push(quote! {
            let key = v8::String::new(scope, #field_name_str).unwrap();
            let value = ::neptune_runtime::extension::IntoV8::into_v8(self.#field_name, scope)?;
            obj.set(scope, key.into(), value);
        });
    }

    let generics = &input.generics;
    let mut impl_generics_with_s = generics.clone();
    if !impl_generics_with_s.params.iter().any(|p| matches!(p, syn::GenericParam::Lifetime(l) if l.lifetime.ident == "s")) {
        impl_generics_with_s.params.push(syn::parse_quote!('s));
    }
    let (impl_generics, _, _) = impl_generics_with_s.split_for_impl();
    let (_, ty_generics, where_clause) = generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics ::neptune_runtime::extension::IntoV8<'s> for #name #ty_generics #where_clause {
            fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, ::neptune_runtime::extension::NeptuneError> {
                let obj = v8::Object::new(scope);
                
                #(#field_writes)*

                Ok(obj.into())
            }
        }
    };

    expanded.into()
}
