/** @type {(s: string) => Uint8Array} */
let textEncodeImpl = globalThis.bootstrap.textEncodeImpl
/** @type {(s: string, dest: Uint8Array) => [number, number]} */
let textEncodeIntoImpl = globalThis.bootstrap.textEncodeIntoImpl
/** @type {(buf: Uint8Array | ArrayBuffer, label: string, fatal: boolean, ignoreBOM: boolean)} */
let textDecodeImpl = globalThis.bootstrap.textDecodeImpl
/** @type {(label: string) => string} */
let textDecodeParseLabel = globalThis.bootstrap.textDecodeParseLabel

class TextEncoder {
    constructor () {}

    get encoding () {
        return "utf-8"
    }

    encode(input = "") {
        return textEncodeImpl(String(input));
    }

    encodeInto(input, destination) {
        if (!(destination instanceof Uint8Array)) {
            throw new TypeError("The destination must be a Uint8Array");
        }
        
        const result = textEncodeIntoImpl(String(input), destination);
        
        return {
            read: result[0],
            written: result[1]
        };
    }
}

class TextDecoder {
    #encoding;
    #fatal;
    #ignoreBOM;

    constructor(label = "utf-8", options = {}) {
        this.#encoding = (label === "utf-8" || label === "utf8" || label === "unicode-1-1-utf-8") ? "utf-8" : textDecodeParseLabel(label)
        this.#fatal = !!options.fatal;
        this.#ignoreBOM = !!options.ignoreBOM;
    }

    get encoding() { return this.#encoding }
    get fatal() { return this.#fatal }
    get ignoreBOM() { return this.#ignoreBOM }

    decode(input) {
        let view;
        if (input instanceof ArrayBuffer) { 
            view = input 
        }
        else { 
            view = input instanceof Uint8Array ? input : new Uint8Array(input.buffer || input); 
        }
        return textDecodeImpl(view, this.#encoding, this.#fatal, this.#ignoreBOM);
    }
}

globalThis.TextEncoder = TextEncoder
globalThis.TextDecoder = TextDecoder