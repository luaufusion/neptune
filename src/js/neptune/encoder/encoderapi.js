/** @type {(s: string) => Uint8Array} */
let textEncodeImpl = globalThis.bootstrap.textEncodeImpl
/** @type {(s: string, dest: Uint8Array) => [number, number]} */
let textEncodeIntoImpl = globalThis.bootstrap.textEncodeIntoImpl

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

globalThis.TextEncoder = TextEncoder