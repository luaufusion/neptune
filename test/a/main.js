console.log("GO")
let timer = new Ticker(10); 
await timer.tick()
console.log("a")
let r = await timer.tick()
console.log("b", r)