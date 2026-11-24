// RustyGB WebAssembly front-end.
//
// Loads the wasm-pack output from ./pkg, wires keyboard input to the
// emulator, draws each frame onto a 160x144 canvas and plays APU samples
// through a ScriptProcessorNode.

import init, { WasmGameBoy, WasmButton } from "../pkg/rusty_gb.js";

const SCREEN_WIDTH = 160;
const SCREEN_HEIGHT = 144;
// 70224 T-cycles at 4194304 Hz.
const FRAME_INTERVAL_MS = (1000 * 70224) / 4194304;
const MAX_LAG_MS = 100;

const canvas = document.getElementById("screen");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const resetBtn = document.getElementById("reset-btn");
const pauseBtn = document.getElementById("pause-btn");
const romInput = document.getElementById("rom-input");

let gb = null;
let lastRom = null;
let running = false;
let frameRequest = null;
let lastTickMs = 0;
let pendingMs = 0;
let audioCtx = null;
let audioNode = null;
const SAMPLE_RATE = 48000;

const KEY_MAP = {
    "KeyZ": WasmButton.A,
    "KeyX": WasmButton.B,
    "Enter": WasmButton.Start,
    "Space": WasmButton.Select,
    "ArrowRight": WasmButton.Right,
    "ArrowLeft": WasmButton.Left,
    "ArrowUp": WasmButton.Up,
    "ArrowDown": WasmButton.Down,
};

function setStatus(message) {
    statusEl.textContent = message;
}

function blankScreen() {
    ctx.fillStyle = "#0f380f";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
}

async function ensureAudio() {
    if (audioCtx) {
        return;
    }
    audioCtx = new (window.AudioContext || window.webkitAudioContext)({
        sampleRate: SAMPLE_RATE,
    });
    // A worklet cannot reach the emulator, which lives on this thread.
    const bufferSize = 1024;
    audioNode = audioCtx.createScriptProcessor(bufferSize, 0, 1);
    audioNode.onaudioprocess = (event) => {
        const out = event.outputBuffer.getChannelData(0);
        if (!gb) {
            out.fill(0);
            return;
        }
        const samples = gb.drain_audio(bufferSize);
        const n = samples.length;
        for (let i = 0; i < n; i++) {
            out[i] = samples[i] * 0.1;
        }
        for (let i = n; i < bufferSize; i++) {
            out[i] = 0;
        }
    };
    audioNode.connect(audioCtx.destination);
    if (audioCtx.state === "suspended") {
        await audioCtx.resume();
    }
}

function renderFrame() {
    const frame = gb.frame();
    const img = new ImageData(
        new Uint8ClampedArray(frame.buffer, frame.byteOffset, frame.byteLength),
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
    );
    ctx.putImageData(img, 0, 0);
}

function tick(now) {
    frameRequest = requestAnimationFrame(tick);
    // Drop time after a stall (e.g. a hidden tab) instead of replaying it.
    pendingMs = Math.min(pendingMs + now - lastTickMs, MAX_LAG_MS);
    lastTickMs = now;
    if (pendingMs < FRAME_INTERVAL_MS) {
        return;
    }
    while (pendingMs >= FRAME_INTERVAL_MS) {
        pendingMs -= FRAME_INTERVAL_MS;
        gb.run_frame();
    }
    renderFrame();
}

function startLoop() {
    if (frameRequest !== null) {
        return;
    }
    pendingMs = 0;
    lastTickMs = performance.now();
    frameRequest = requestAnimationFrame(tick);
}

function stopLoop() {
    if (frameRequest !== null) {
        cancelAnimationFrame(frameRequest);
        frameRequest = null;
    }
}

async function loadRom(bytes, name) {
    await ensureAudio();
    // Build first: a rejected ROM must leave the running game usable.
    const next = new WasmGameBoy(bytes, SAMPLE_RATE);
    gb?.free();
    gb = next;
    lastRom = bytes;
    running = true;
    resetBtn.disabled = false;
    pauseBtn.disabled = false;
    pauseBtn.textContent = "Pause";
    startLoop();
    setStatus(`Playing: ${name}`);
}

async function tryLoadRom(bytes, name) {
    try {
        await loadRom(bytes, name);
    } catch (e) {
        setStatus(`Failed to load ROM: ${e}`);
    }
}

async function tryLoadFile(file) {
    if (!file) {
        return;
    }
    try {
        await tryLoadRom(new Uint8Array(await file.arrayBuffer()), file.name);
    } catch (e) {
        setStatus(`Failed to read ROM: ${e}`);
    }
}

romInput.addEventListener("change", (event) => tryLoadFile(event.target.files?.[0]));

canvas.addEventListener("dragover", (event) => event.preventDefault());
canvas.addEventListener("drop", (event) => {
    event.preventDefault();
    tryLoadFile(event.dataTransfer?.files?.[0]);
});

resetBtn.addEventListener("click", () => {
    if (lastRom) {
        tryLoadRom(lastRom, "(reset)");
    }
});

pauseBtn.addEventListener("click", () => {
    if (!gb) {
        return;
    }
    running = !running;
    pauseBtn.textContent = running ? "Pause" : "Resume";
    if (running) {
        startLoop();
    } else {
        stopLoop();
    }
});

window.addEventListener("keydown", (event) => {
    const btn = KEY_MAP[event.code];
    if (btn === undefined || !gb) {
        return;
    }
    event.preventDefault();
    gb.set_button(btn, true);
});

window.addEventListener("keyup", (event) => {
    const btn = KEY_MAP[event.code];
    if (btn === undefined || !gb) {
        return;
    }
    event.preventDefault();
    gb.set_button(btn, false);
});

(async function bootstrap() {
    try {
        await init();
        blankScreen();
    } catch (e) {
        setStatus(`Failed to initialise WebAssembly module: ${e}`);
    }
})();
