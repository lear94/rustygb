# RustyGB — WebAssembly front-end

This directory hosts a minimal browser UI that drives the RustyGB core
through its `wasm-bindgen` bindings (see `src/wasm.rs`).

## Build

You need [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) on your `PATH`:

```bash
cargo install wasm-pack
```

From the project root, build the WebAssembly module:

```bash
wasm-pack build --release --target web --out-dir pkg
```

The command produces `pkg/rusty_gb.js` and `pkg/rusty_gb_bg.wasm`, which the
HTML page imports through an ES module loader.

## Serve

Browsers require WebAssembly modules to be served over HTTP, not opened as
local files. Any static server works; the simplest is the one bundled with
Python:

```bash
python3 -m http.server 8000
```

Then open <http://localhost:8000/web/>, click **Load ROM**, and pick a
`.gb` cartridge image you legally own.

## Controls

| Game Boy | Key       |
|----------|-----------|
| A        | `Z`       |
| B        | `X`       |
| Select   | `Space`   |
| Start    | `Enter`   |
| D-Pad    | Arrow keys |

## Notes

* Save files are not persisted across reloads (the WebAssembly build has no
  filesystem access). Hooking the cartridge RAM up to IndexedDB or
  `localStorage` is left as a small follow-up.
* The current audio path uses `ScriptProcessorNode` for portability.
  Switching to an `AudioWorklet` would shave a few milliseconds of latency
  on modern browsers.
