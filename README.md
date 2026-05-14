# Neptune

Neptune is the work-in-progress experimental JS runtime for cases that need strict sandboxing+hardening (untrusted code execution etc.)

## Supported/Implemented APIs

- `setTimeout` / `setInterval` / `clearTimeout` / `clearInterval`
- `performance.now`
- `console.log`/`console.log` (rest of Console API is WIP)
- `structuredClone` (partial, DOMException not yet supported)
- `TextEncoder`, `TextDecoder`

## Embedder Pipe API

To allow for communication between the embedder host and the underlying javascript worker, Neptune offers the "Neptune Embedder Pipe API" as follows:

**JS Side**

- `postMessage(msg: string | ArrayBuffer | ArrayBufferView)` => Pushes a message `msg` to the embedder synchronously
- `setMessageCallback(f: (string | ArrayBuffer): any)` => Sets the message callback to `f`. Whenever an embedder sends a message to the worker, the callback `f` will be called with the sent message

**Embedder Side**

- `JsRuntime::push_message` => Allows pushing either a `string` or an `Vec<u8>` (copied into a JS ArrayBuffer) to the worker
- `JsRuntime::set_worker_to_embedder_cb` => Allows for the embedder to recieve a callback when the worker sends an embedder to it
- `JsRuntime::set_embedder_to_worker_cb` => Allows for the embedder to override the callback the worker gave with `setMessageCallback`. Should not be used outside of debugging and may be removed

## TODO APIS

- `btoa`, `atob`