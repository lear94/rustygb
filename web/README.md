# RustyGB — WebAssembly front-end

This directory hosts a minimal browser UI that drives the RustyGB core
through its `wasm-bindgen` bindings (see `wasm/`).

## Build

You need [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) on your `PATH`:

```bash
cargo install wasm-pack
```

From the project root, build the WebAssembly module:

```bash
wasm-pack build wasm --release --target web --out-dir ../pkg --out-name rusty_gb
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

* Battery saves are kept in `localStorage`, per ROM image, every few seconds and when
  the page closes. Clearing site data deletes them.
* The current audio path uses `ScriptProcessorNode` because an `AudioWorklet`
  cannot reach the emulator, which runs on the main thread.
