// RustyGB WebAssembly front-end.
//
// Loads the wasm-pack output from ../pkg, wires keyboard input to the
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
// Storage key of the running game and what it held when this tab last used it.
let save = null;
let lastName = "";
let loadSequence = 0;
let running = false;
let frameRequest = null;
let lastTickMs = 0;
let pendingMs = 0;
let audioCtx = null;
let audioNode = null;
const SAMPLE_RATE = 48000;
const MAX_LATENCY_SAMPLES = 4800;
const AUTOSAVE_INTERVAL_MS = 5000;

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

function ensureAudio() {
    if (!audioCtx) {
        const context = new (window.AudioContext || window.webkitAudioContext)({
            sampleRate: SAMPLE_RATE,
        });
        // A worklet cannot reach the emulator, which lives on this thread.
        const bufferSize = 1024;
        const node = context.createScriptProcessor(bufferSize, 0, 1);
        node.onaudioprocess = (event) => playSamples(event.outputBuffer.getChannelData(0));
        node.connect(context.destination);
        audioCtx = context;
        audioNode = node;
    }
    if (audioCtx.state === "suspended") {
        // Pends until the browser allows audio; the game must not wait for it.
        audioCtx.resume().catch(() => {});
    }
}

// The game runs without audio when the browser refuses it.
function tryAudio() {
    try {
        ensureAudio();
    } catch (e) {
        console.warn("Audio unavailable:", e);
    }
}

function playSamples(out) {
    out.fill(0);
    if (!gb || !running) {
        return;
    }
    // Samples queued while audio was suspended would become permanent latency.
    const excess = gb.buffered_audio() - MAX_LATENCY_SAMPLES;
    if (excess > 0) {
        gb.drain_audio(excess);
    }
    out.set(gb.drain_audio(out.length));
}

// Title, cartridge type and checksums. Hacks often keep them, so this alone is not a key.
function headerKey(rom) {
    const header = Array.from(rom.subarray(0x134, 0x150));
    return "rustygb:" + header.map((b) => b.toString(16).padStart(2, "0")).join("");
}

// FNV-1a over the whole image; not cryptographic, only to tell ROMs apart.
function saveKey(rom) {
    let hash = 0x811c9dc5;
    for (const byte of rom) {
        hash = Math.imul(hash ^ byte, 0x01000193);
    }
    return `${headerKey(rom)}:${rom.length}:${(hash >>> 0).toString(16)}`;
}

function decodeSave(text) {
    return text === null ? undefined : Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
}

// What storage holds for a ROM. Blocked storage must not stop the game.
function openSave(rom) {
    const key = saveKey(rom);
    const legacyKey = headerKey(rom);
    try {
        const text = localStorage.getItem(key);
        if (text !== null) {
            return { key, text, legacy: null };
        }
        // Saves used to be keyed by header only, which hacks share with their original.
        const legacy = localStorage.getItem(legacyKey);
        const adopt =
            legacy !== null && confirm("Use the save found for a game with the same header?");
        return { key, text, legacy: adopt ? { key: legacyKey, text: legacy } : null };
    } catch (e) {
        console.warn("Saves unavailable:", e);
        return { key, text: null, legacy: null };
    }
}

// Returns whether no progress is pending.
function persist() {
    if (!gb || !gb.needs_save()) {
        return true;
    }
    const image = gb.save_image(unixNow());
    try {
        const current = localStorage.getItem(save.key);
        // Cleared site data holds nothing to lose.
        if (current !== null && current !== save.text) {
            throw new Error("another tab saved this game");
        }
        const text = btoa(Array.from(image, (b) => String.fromCharCode(b)).join(""));
        localStorage.setItem(save.key, text);
        save.text = text;
        claimLegacySave();
    } catch (e) {
        setStatus(`SAVE FAILED: ${e.message ?? e}`);
        return false;
    }
    gb.mark_saved();
    return true;
}

// Once copied under its own key, an adopted save must not be offered to other ROMs.
function claimLegacySave() {
    const legacy = save.legacy;
    save.legacy = null;
    if (legacy && localStorage.getItem(legacy.key) === legacy.text) {
        localStorage.removeItem(legacy.key);
    }
}

function unixNow() {
    return Math.floor(Date.now() / 1000);
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

function loadRom(bytes, name) {
    tryAudio();
    if (!persist() && !confirm("Saving failed. Lose the current progress?")) {
        return;
    }
    // Build first: a rejected ROM must leave the running game usable.
    const opened = openSave(bytes);
    const image = decodeSave(opened.legacy?.text ?? opened.text);
    const next = new WasmGameBoy(bytes, image, SAMPLE_RATE, unixNow());
    gb?.free();
    gb = next;
    lastRom = bytes;
    save = opened;
    lastName = name;
    running = true;
    resetBtn.disabled = false;
    pauseBtn.disabled = false;
    pauseBtn.textContent = "Pause";
    startLoop();
    setStatus(`Playing: ${name}`);
}

function tryLoadRom(bytes, name) {
    try {
        loadRom(bytes, name);
    } catch (e) {
        setStatus(`Failed to load ROM: ${e}`);
    }
}

async function tryLoadFile(file) {
    if (!file) {
        return;
    }
    loadSequence += 1;
    const sequence = loadSequence;
    try {
        const [buffer] = await Promise.all([file.arrayBuffer(), ready]);
        // A later request wins even if its file is read first.
        if (sequence === loadSequence) {
            tryLoadRom(new Uint8Array(buffer), file.name);
        }
    } catch (e) {
        setStatus(`Failed to read ROM: ${e}`);
    }
}

romInput.addEventListener("change", () => {
    romInput.blur();
    tryLoadFile(romInput.files?.[0]);
    // Otherwise choosing the same file again fires no event.
    romInput.value = "";
});

// On the whole window: a missed drop would navigate away from the game.
window.addEventListener("dragover", (event) => event.preventDefault());
window.addEventListener("drop", (event) => {
    event.preventDefault();
    tryLoadFile(event.dataTransfer?.files?.[0]);
});

resetBtn.addEventListener("click", () => {
    resetBtn.blur();
    if (lastRom) {
        tryLoadRom(lastRom, lastName);
    }
});

pauseBtn.addEventListener("click", () => {
    pauseBtn.blur();
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

function setKey(event, pressed) {
    const btn = KEY_MAP[event.code];
    if (btn === undefined || !gb) {
        return;
    }
    // Enter and Space belong to a focused control; a release still reaches the game.
    if (event.target instanceof Element && event.target.closest("button, input")) {
        if (!pressed) {
            gb.set_button(btn, false);
        }
        return;
    }
    if (pressed && (event.ctrlKey || event.metaKey || event.altKey)) {
        return;
    }
    event.preventDefault();
    gb.set_button(btn, pressed);
}

function releaseAllKeys() {
    if (gb) {
        Object.values(KEY_MAP).forEach((btn) => gb.set_button(btn, false));
    }
}

window.addEventListener("keydown", (event) => {
    if (gb) {
        tryAudio();
    }
    setKey(event, true);
});
window.addEventListener("keyup", (event) => setKey(event, false));
window.addEventListener("pointerdown", () => {
    if (gb) {
        tryAudio();
    }
});
window.addEventListener("blur", releaseAllKeys);
window.addEventListener("pagehide", persist);
window.addEventListener("beforeunload", (event) => {
    if (!persist()) {
        event.preventDefault();
    }
});
document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
        releaseAllKeys();
        persist();
    }
});
setInterval(persist, AUTOSAVE_INTERVAL_MS);

const ready = init();
ready.then(blankScreen, (e) => setStatus(`Failed to initialise WebAssembly module: ${e}`));
