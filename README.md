# Neptune

Neptune is the work-in-progress experimental JS runtime for cases that need strict sandboxing+hardening (untrusted code execution etc.)

## Supported/Implemented APIs

- `setTimeout` / `setInterval` / `clearTimeout` / `clearInterval`
- `console.log` (only basic logging, rest of Console API is WIP)
- `structuredClone` (partial, DOMException not yet supported)