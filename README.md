# plexus-macros

#[activation]: a Rust module becomes a WebAssembly component, its WIT written from the code, and natively a process.

The module is read into an IR (`plexus-ir`); its WIT and TypeScript are written from the IR by `plexus-gen`. One file builds two ways: to `wasm32` a component (wit-bindgen's exports and types); natively its own types with serde's derives by the value layout (`plexus-proto`'s `value`), and `serve(activation, server, at)` onto a `plexus::Server`: each body read by name (a wrong one fails `invalid-params`, naming the parameter, before the method runs), `async fn` awaited, a stream paced by the server, a typed failure written as its case's name and value. The code it writes names `::plexus` and nothing else of ours. `#[activation(describe)]` only describes.

Part of Plexus, built from scratch (tracker: LAYERS-9). Its sibling crates are expected beside it: `../<crate>`.

MIT.
