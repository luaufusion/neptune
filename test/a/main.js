import * as a from "b.js"
console.log("GO")
let timer = new Ticker(10000); 
let r0 = await timer.tick()
console.log("a", r0)
let r = await timer.tick()
console.log("b", r)
