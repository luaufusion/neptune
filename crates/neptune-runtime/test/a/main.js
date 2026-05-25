// Base test =================
import * as a from "./b.js"

console.log("GO", Reflect, eval, {a: "123"}, structuredClone(new DOMException("abc", "123")))
console.log(Uint8Array, structuredClone, encodeURIComponent, decodeURIComponent, String.fromCodePoint, Math, atob, btoa, Uint8Array.fromBase64)
console.log({a:1}.a, structuredClone({a:1}) == {a:1})
console.log(new Promise((resolve) => resolve(123)))
console.log(Math.random())

// Encoder API (Encode)
let s = "hello world"
let enc = new TextEncoder()
let dec = new TextDecoder()
let ui8 = enc.encode(s)
console.log(ui8, dec.decode(ui8))
enc.encodeInto("bob", ui8)
console.log(ui8, dec.decode(ui8))
console.log(Intl, Temporal)

// Message test =============
const cb = (msg) => {
    console.log(`[Worker] ${msg}`)
    setMessageCallback(undefined)
    if(getMessageCallback() !== null) throw new Error("getMessageCallback did not return undefined after setting to undefined")
}
setMessageCallback(cb)
if(getMessageCallback() !== cb) throw new Error("getMessageCallback did not return the expected cb")
postMessage("1234")
let ab = new ArrayBuffer(8);
let view = new Uint8Array(ab);
view[1] = 2
postMessage(view)

// async test ================
console.log("sleep")
let sleepRet = await testSleepAsync(5)
console.log("sleep done", sleepRet)

// Timer test ================
class Ticker {
    constructor(n) {
        this.n = n
        this.timerId = null;
    }

    async tick() {
        return new Promise((resolve) => {
            this.timerId = setTimeout(resolve, this.n, 123);
        });
    }

    stop() {
        if (this.timerId) {
            clearTimeout(this.timerId);
            this.timerId = null;
        }
    }
}

let timer = new Ticker(10000); 

// Start intervaling
let count = 0
let tn = performance.now()
let cid = setInterval(() => {
    let oldTn = tn
    tn = performance.now()
    console.log(`setInterval(${cid}, ${count}, ${oldTn - tn})`)
    count++
    if (count > 5) {
        clearInterval(cid)
        postMessage("Got here")
        //timer.stop()
    }
}, 1000)

// rest of tests
let r0 = await timer.tick()
console.log("a", r0, performance.now())
let r = await timer.tick()
console.log("b", r, performance.now())
