// RustyGB WebAssembly front-end.
//
// Loads the wasm-pack output from ./pkg, wires keyboard input to the
// emulator, draws each frame onto a 160x144 canvas and streams APU samples
// to an AudioWorklet for low-latency playback.

import init, { WasmGameBoy, WasmButton } from "../pkg/rusty_gb.js";

const SCREEN_WIDTH = 160;
const SCREEN_HEIGHT = 144;
const FRAME_INTERVAL_MS = 1000 / 59.73;

const canvas = document.getElementById("screen");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const resetBtn = document.getElementById("reset-btn");
const pauseBtn = document.getElementById("pause-btn");
const romInput = document.getElementById("rom-input");

let gb = null;
let lastRom = null;
let running = false;
let frameTimer = null;
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
    // Lightweight script-processor fallback: pull samples from the
    // emulator on each audio callback. A more sophisticated build could
    // swap this out for an AudioWorklet.
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

function tick() {
    if (!gb || !running) {
        return;
    }
    gb.run_frame();
    renderFrame();
}

function startLoop() {
    if (frameTimer !== null) {
        return;
    }
    frameTimer = setInterval(tick, FRAME_INTERVAL_MS);
}

function stopLoop() {
    if (frameTimer !== null) {
        clearInterval(frameTimer);
        frameTimer = null;
    }
}

async function loadRom(bytes, name) {
    await ensureAudio();
    gb?.free?.();
    gb = new WasmGameBoy(bytes, SAMPLE_RATE);
    lastRom = bytes;
    running = true;
    resetBtn.disabled = false;
    pauseBtn.disabled = false;
    pauseBtn.textContent = "Pause";
    startLoop();
    setStatus(`Playing: ${name}`);
}

romInput.addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file) {
        return;
    }
    const bytes = new Uint8Array(await file.arrayBuffer());
    try {
        await loadRom(bytes, file.name);
    } catch (e) {
        setStatus(`Failed to load ROM: ${e}`);
    }
});

canvas.addEventListener("dragover", (event) => event.preventDefault());
canvas.addEventListener("drop", async (event) => {
    event.preventDefault();
    const file = event.dataTransfer?.files?.[0];
    if (!file) {
        return;
    }
    const bytes = new Uint8Array(await file.arrayBuffer());
    try {
        await loadRom(bytes, file.name);
    } catch (e) {
        setStatus(`Failed to load ROM: ${e}`);
    }
});

resetBtn.addEventListener("click", async () => {
    if (!lastRom) {
        return;
    }
    await loadRom(lastRom, "(reset)");
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
