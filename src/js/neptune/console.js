let neptunePrint = globalThis.bootstrap.consoleLog
let getPromiseDetails = globalThis.bootstrap.getPromiseDetails

const ANSI = {
  reset: "\x1b[0m",
  number: "\x1b[33m", // Yellow
  string: "\x1b[32m", // Green
  boolean: "\x1b[35m", // Magenta
  cyan: "\x1b[36m",
  dim: "\x1b[90m"
}

function inspect(val, depth = 4, seen = new WeakSet()) {
    if (val === null) return `${ANSI.cyan}null${ANSI.reset}`;
    if (val === undefined) return `${ANSI.dim}undefined${ANSI.reset}`;
  
    const type = typeof val;
  
    if (type === 'number') return `${ANSI.number}${val}${ANSI.reset}`;
    if (type === 'boolean') return `${ANSI.boolean}${val}${ANSI.reset}`;
    if (type === 'string') {
        if(depth == 4) return `${val}`
        return `${ANSI.string}'${val}'${ANSI.reset}`
    };
    if (type === "bigint") return `${ANSI.number}BigInt(${val})${ANSI.reset}`
    if (type === "symbol") return `${ANSI.number}Symbol('${val}')${ANSI.reset}`
    if (type == 'function') return `${ANSI.cyan}[Function: ${val.name}]${ANSI.reset}`
  
    if (type === 'object') {
        if (seen.has(val)) return `${ANSI.cyan}[Circular]${ANSI.reset}`;
        if (depth < 0) return `${ANSI.dim}[Object]${ANSI.reset}`;

        if(val instanceof Date) {
            return `[Date ${val.toISOString()}]`
        } else if (val instanceof Promise) {
            let [state, resolveVal] = getPromiseDetails(val)
            switch (state) {
                case 0:
                    return `[Promise [pending]]`
                case 1:
                    return `[Promise ${inspect(resolveVal, depth - 1, seen)}]`
                case 2:
                    return `[Promise <rejected> ${inspect(resolveVal, depth - 1, seen)}]`
            }
            return `[Promise <${val.state}>]`
        }
    
        seen.add(val);
    
        if (ArrayBuffer.isView(val)) {
            return `${val.constructor.name}(${val.length}) [ ${Array.from(val.buffer.slice(0, 10)).join(', ')}${val.buffer.byteLength > 10 ? '...' : ''} ]`;
        }

        if (Array.isArray(val)) {
            const items = val.map(i => inspect(i, depth - 1, seen));
            return `[ ${items.join(", ")} ]`;
        }

        const entries = Reflect.ownKeys(Object.getOwnPropertyDescriptors(val)).map((k) => `${String(k)}: ${inspect(val[k], depth - 1, seen)}`);
        return `${val} { ${entries.join(", ")} }`;
    }

  return String(val);
}

globalThis.console = {
    log: function(...args) {
        const formatted = args.map(arg => inspect(arg)).join(" ");
        neptunePrint(0, formatted)        
    },
    error: function(...args) {
        const formatted = args.map(arg => inspect(arg)).join(" ");
        neptunePrint(1, formatted)        
    },
    [Symbol.toStringTag]: 'console'
};