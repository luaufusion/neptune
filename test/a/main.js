// Base test =================
import * as a from "./b.js"
console.log("GO", Reflect, eval)
console.log(Uint8Array)

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
        //timer.stop()
    }
}, 1000)

// rest of tests
let r0 = await timer.tick()
console.log("a", r0)
let r = await timer.tick()
console.log("b", r)