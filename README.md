# plexus-macros

#[activation]: a Rust module becomes a WebAssembly component, its WIT written from the code, and natively a process.

The module is read into an IR (`plexus-ir`); its WIT and TypeScript are written from the IR by `plexus-gen`. One file builds two ways: to `wasm32` a component (wit-bindgen's exports and types); natively its own types with serde's derives by the value layout (`plexus-proto`'s `value`), and `serve(activation, server, at)` onto a `plexus::Server`: each body read by name (a wrong one fails `invalid-params`, naming the parameter, before the method runs), `async fn` awaited, a stream paced by the server, a typed failure written as its case's name and value. The code it writes names `::plexus` and nothing else of ours. A `#[child]` is served natively under `<at>.<child>`: one returned by reference (`&Kiln`, an activation written with `#[activation]`, here or in another crate) runs in this process on its parent's thread; one that is anything (`Box<dyn Activation>`) or fits an interface (`Box<dyn Glazes>`, where `trait Glazes: plexus::Activation`) is asked for once, when its parent is served, and mounted. A child per key (`list`, `search`), or one that may fail, is a compile error natively; a component holds no children yet. `#[activation(describe)]` only describes.

Native methods can take `&plexus::Caller` to require an authenticated caller or `Option<&plexus::Caller>` to admit anonymous calls. Use `#[plexus::activation(context = StudioCx)]` with a named struct inside the module and `#[derive(plexus::Context)]`. Fields marked `#[from(caller)]` receive the caller or resolve an application type through `FromCaller<Activation>`. Other fields parse named keys on the activation's path segment using their `FromStr` implementations, as in `studio[tenant=acme].status`. An optional `#[open] fn open(&self, cx: &StudioCx) -> Result<(), Denied>` checks every unary method and stream before it runs, including methods that take no context argument. Caller and context arguments are omitted from wire parameters. A contextual activation can serve fixed native children returned by reference and remote children returned as boxed activations. Calls to descendants, including children mounted during server setup, resolve the ancestor context from the original caller and the ancestor’s path keys, and run its open check before entering the child. With multiple contextual ancestors, the checks run from outermost to innermost. The child factory takes no caller or context. Caller-aware and contextual activations require a native build.

A native method may accept one owned `Input<T>` alongside its ordinary arguments.
It implements `Stream<Item = Result<T, plexus::Failure>>`. Input items arrive as
the handler reads them. For `Input<Bytes>`, including aliases of bytes, the handler
receives the same byte vectors from raw frames and from JSON/base64 input.
An input method can return a single result or stream its
own items back. The generated WIT describes the input as `stream<T>`; the Rust
component adapter for receiving input requires a future implementation.

Part of Plexus, built from scratch (tracker: LAYERS-9). Its sibling crates are expected beside it: `../<crate>`.

MIT.
