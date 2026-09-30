<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Camera, cameraErrorMessage } from '../lib/camera'
import AppIcon from '../components/AppIcon.vue'

// The kiosk is a "dumb" device: it captures frames and shows what the server
// decides. It never guesses an identity itself.
//
// Protocol (see crates/server/src/routes/ws.rs):
//   -> {"type":"hello","device_token":"..."}
//   -> {"type":"frame","image":"<base64 jpeg>"}
//   <- {"type":"ready", ...} | {"type":"result", ...} | {"type":"error", ...}

interface StudentView {
  id: string
  nama: string
  nis: string
}

interface ResultMessage {
  type: 'result'
  kind: 'continue' | 'accepted' | 'rejected'
  prompt?: string
  message?: string
  student?: StudentView
  similarity?: number
  already_marked?: boolean
  frames_seen?: number
}

type ServerMessage =
  | { type: 'ready'; device_id: string; device_name: string; gallery_students: number }
  | ResultMessage
  | { type: 'error'; message: string }

const TOKEN_KEY = 'absensi.device_token'
const FRAME_INTERVAL_MS = 250

const videoEl = ref<HTMLVideoElement | null>(null)
const camera = new Camera()

const deviceToken = ref(localStorage.getItem(TOKEN_KEY) ?? '')
const tokenInput = ref('')
const connected = ref(false)
const status = ref<'idle' | 'connecting' | 'live' | 'error'>('idle')
const errorMsg = ref('')
const cameraError = ref('')

const deviceName = ref('')
const galleryCount = ref(0)
const prompt = ref('Hadapkan wajah ke kamera')
const framesSeen = ref(0)

// The last accepted student, shown large for confirmation.
const celebration = ref<StudentView | null>(null)
const celebrationNote = ref('')
const celebrationAt = ref(0)

let ws: WebSocket | null = null
let frameTimer: number | null = null
let reconnectTimer: number | null = null
let celebrationTimer: number | null = null
// True while we intend to hold a connection. Cleared only by an explicit
// teardown (forget device / unmount), so an unexpected close always retries.
let wantsConnection = false
let reconnectAttempts = 0

const clock = ref('')
let clockTimer: number | null = null

const ts = computed(() => new Date(celebrationAt.value).toLocaleTimeString('id-ID', { hour: '2-digit', minute: '2-digit' }))

onMounted(async () => {
  updateClock()
  clockTimer = window.setInterval(updateClock, 1000)
  if (deviceToken.value) {
    await begin()
  } else {
    status.value = 'idle'
  }
})

onBeforeUnmount(() => {
  teardown()
  if (clockTimer) clearInterval(clockTimer)
})

function updateClock(): void {
  clock.value = new Date().toLocaleTimeString('id-ID', { hour: '2-digit', minute: '2-digit' })
}

async function startCamera(): Promise<void> {
  cameraError.value = ''
  if (!videoEl.value) return
  try {
    await camera.start(videoEl.value, { width: 640, height: 480 })
  } catch (e) {
    cameraError.value = cameraErrorMessage(e)
  }
}

async function saveToken(): Promise<void> {
  const t = tokenInput.value.trim()
  if (!t) return
  localStorage.setItem(TOKEN_KEY, t)
  deviceToken.value = t
  await begin()
}

async function begin(): Promise<void> {
  // Fresh start (mount or a new token): reset the backoff counter.
  reconnectAttempts = 0
  await startCamera()
  connect()
}

function connect(): void {
  teardownSocket()
  wantsConnection = true
  status.value = 'connecting'
  errorMsg.value = ''

  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  ws = new WebSocket(`${proto}://${location.host}/ws/kiosk`)

  ws.onopen = () => {
    reconnectAttempts = 0
    ws?.send(JSON.stringify({ type: 'hello', device_token: deviceToken.value }))
  }

  ws.onmessage = (ev) => {
    let msg: ServerMessage
    try {
      msg = JSON.parse(ev.data)
    } catch {
      return
    }
    handleMessage(msg)
  }

  ws.onclose = () => {
    connected.value = false
    stopFrames()
    // Retry on ANY unexpected close. The earlier guard skipped retries once
    // `status` was 'error', but `onerror` always fires before `onclose`, so the
    // kiosk stayed dead after a single network blip. Backoff grows to 15s.
    if (wantsConnection) {
      reconnectAttempts += 1
      const delay = Math.min(3000 * reconnectAttempts, 15000)
      // Clear any already-pending retry first: onclose can fire more than once
      // (or after a manual "Coba lagi"), and stacking timers would open several
      // sockets at once and double the frame rate.
      if (reconnectTimer) clearTimeout(reconnectTimer)
      reconnectTimer = window.setTimeout(connect, delay)
    }
  }

  ws.onerror = () => {
    // Do not set a terminal state here: onclose owns the retry decision.
    errorMsg.value = 'Koneksi ke server gagal. Mencoba lagi…'
    status.value = 'error'
  }
}

function handleMessage(msg: ServerMessage): void {
  switch (msg.type) {
    case 'ready':
      connected.value = true
      status.value = 'live'
      deviceName.value = msg.device_name
      galleryCount.value = msg.gallery_students
      prompt.value = 'Hadapkan wajah ke kamera'
      startFrames()
      break
    case 'result':
      handleResult(msg)
      break
    case 'error':
      // Before the first `ready` this is a real connection/handshake failure.
      // After that it is a per-frame server error (bad frame, inference
      // failure): show it as a transient prompt so the user is not left staring
      // at a stale "hadapkan wajah" message with no feedback.
      if (!connected.value) {
        status.value = 'error'
        errorMsg.value = msg.message
      } else {
        prompt.value = msg.message || 'Terjadi kesalahan. Coba lagi.'
      }
      break
  }
}

function handleResult(msg: ResultMessage): void {
  if (msg.kind === 'accepted' && msg.student) {
    celebration.value = msg.student
    celebrationNote.value = msg.already_marked ? 'Sudah absen' : 'Absensi tercatat'
    celebrationAt.value = Date.now()
    prompt.value = 'Terima kasih'
    stopFrames()
    // Pause capture briefly so the person reads their name, then resume.
    if (celebrationTimer) clearTimeout(celebrationTimer)
    celebrationTimer = window.setTimeout(() => {
      celebration.value = null
      prompt.value = 'Hadapkan wajah ke kamera'
      startFrames()
    }, 3500)
    return
  }
  if (msg.kind === 'rejected') {
    prompt.value = msg.message ?? 'Tidak dikenali. Coba lagi atau hubungi guru.'
    return
  }
  // continue
  prompt.value = msg.prompt ?? prompt.value
  framesSeen.value = msg.frames_seen ?? framesSeen.value
}

function startFrames(): void {
  if (frameTimer !== null) return
  frameTimer = window.setInterval(sendFrame, FRAME_INTERVAL_MS)
}

function stopFrames(): void {
  if (frameTimer !== null) {
    clearInterval(frameTimer)
    frameTimer = null
  }
}

function sendFrame(): void {
  if (!ws || ws.readyState !== WebSocket.OPEN || celebration.value) return
  const image = camera.capture(0.7)
  if (!image) return
  ws.send(JSON.stringify({ type: 'frame', image }))
}

function teardownSocket(): void {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  if (ws) {
    ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null
    ws.close()
    ws = null
  }
}

function teardown(): void {
  // Stop intending to be connected BEFORE closing, so onclose does not schedule
  // another retry.
  wantsConnection = false
  stopFrames()
  teardownSocket()
  camera.stop()
  if (celebrationTimer) clearTimeout(celebrationTimer)
}

function forgetDevice(): void {
  teardown()
  localStorage.removeItem(TOKEN_KEY)
  deviceToken.value = ''
  status.value = 'idle'
}

function toggleFullscreen(): void {
  if (document.fullscreenElement) document.exitFullscreen()
  else document.documentElement.requestFullscreen().catch(() => {})
}
</script>

<template>
  <!-- Setup screen: no device token yet -->
  <div v-if="!deviceToken" class="kiosk-setup">
    <div class="card card-pad setup-card">
      <span class="brand-mark" aria-hidden="true"><AppIcon name="scan-face" :size="24" /></span>
      <h1>Aktifkan kiosk</h1>
      <p class="muted">
        Tempelkan token perangkat dari dashboard admin. Token hanya perlu dimasukkan
        sekali per perangkat.
      </p>
      <form @submit.prevent="saveToken">
        <div class="field">
          <label class="label" for="tok">Token perangkat</label>
          <input id="tok" v-model="tokenInput" class="input" placeholder="kiosk_…" autocomplete="off" />
        </div>
        <button class="btn btn-primary setup-btn" type="submit">Hubungkan</button>
      </form>
    </div>
  </div>

  <!-- Live kiosk -->
  <div v-else class="kiosk">
    <video ref="videoEl" class="kiosk-video" playsinline muted />

    <div class="kiosk-scrim" />

    <!-- Top status -->
    <header class="kiosk-top">
      <span class="kiosk-brand">
        <AppIcon name="scan-face" :size="22" />
        <span>Absensi</span>
      </span>
      <div class="kiosk-meta">
        <span class="status-line" :class="{ dim: !connected }">
          <span class="dot" :class="connected ? 'dot-success' : status === 'error' ? 'dot-error' : 'dot-warning'" />
          <span>{{ connected ? 'Terhubung' : status === 'connecting' ? 'Menghubungkan…' : status === 'error' ? 'Terputus' : 'Siap' }}</span>
        </span>
        <span class="clock tnum">{{ clock }}</span>
      </div>
    </header>

    <!-- Camera error -->
    <div v-if="cameraError" class="kiosk-center">
      <div class="kiosk-card">
        <AppIcon name="alert" :size="32" />
        <h2>Kamera bermasalah</h2>
        <p>{{ cameraError }}</p>
        <button class="btn btn-secondary" type="button" @click="startCamera">Coba lagi</button>
      </div>
    </div>

    <!-- Connection error -->
    <div v-else-if="status === 'error'" class="kiosk-center">
      <div class="kiosk-card">
        <AppIcon name="alert" :size="32" />
        <h2>Tidak terhubung ke server</h2>
        <p>{{ errorMsg }}</p>
        <div class="row">
          <button class="btn btn-secondary" type="button" @click="connect">Coba lagi</button>
          <button class="btn btn-ghost" type="button" @click="forgetDevice">Ganti token</button>
        </div>
      </div>
    </div>

    <!-- Accepted -->
    <div v-else-if="celebration" class="kiosk-center">
      <div class="kiosk-card celebration">
        <span class="celebration-icon" aria-hidden="true"><AppIcon name="check-circle" :size="48" /></span>
        <h2 class="celebration-name">{{ celebration.nama }}</h2>
        <p class="celebration-nis tnum">{{ celebration.nis }}</p>
        <p class="celebration-note">{{ celebrationNote }} · {{ ts }}</p>
      </div>
    </div>

    <!-- Waiting / prompt -->
    <div v-else class="kiosk-center">
      <div class="kiosk-prompt">
        <div class="scan-ring" :class="{ scanning: connected }" aria-hidden="true" />
        <p class="prompt-text">{{ prompt }}</p>
        <p v-if="framesSeen > 0" class="prompt-sub tnum">menganalisis… ({{ framesSeen }})</p>
        <p v-else class="prompt-sub">{{ galleryCount }} siswa terdaftar</p>
      </div>
    </div>

    <!-- Bottom bar -->
    <footer class="kiosk-bottom">
      <span v-if="deviceName" class="device-name">{{ deviceName }}</span>
      <div class="kiosk-actions">
        <button class="btn btn-ghost btn-sm on-dark" type="button" @click="toggleFullscreen">
          Layar penuh
        </button>
        <button class="btn btn-ghost btn-sm on-dark" type="button" @click="forgetDevice">
          Ganti token
        </button>
      </div>
    </footer>
  </div>
</template>

<style scoped>
/* ---- Setup ---- */
.kiosk-setup {
  min-height: 100%;
  display: grid;
  place-items: center;
  padding: calc(var(--space-xl) + var(--safe-top)) var(--space-xl)
    calc(var(--space-xl) + var(--safe-bottom));
  background: var(--surface-canvas);
}

@supports (min-height: 100dvh) {
  .kiosk-setup {
    min-height: 100dvh;
  }
}

.setup-card {
  width: min(26rem, 100%);
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
  box-shadow: var(--shadow-lg);
}

.brand-mark {
  display: grid;
  place-items: center;
  width: 2.75rem;
  height: 2.75rem;
  border-radius: var(--radius-md);
  background: var(--accent-bg);
  color: var(--accent-text);
}

.setup-card h1 {
  font-size: var(--text-2xl);
}

.setup-btn {
  width: 100%;
  margin-top: var(--space-md);
}

.muted {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

/* ---- Live kiosk ---- */
.kiosk {
  position: fixed;
  inset: 0;
  background: var(--gray-950);
  overflow: hidden;
}

.kiosk-video {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  transform: scaleX(-1);
}

.kiosk-scrim {
  position: absolute;
  inset: 0;
  background: linear-gradient(180deg, var(--overlay-surface) 0%, transparent 22%, transparent 55%, var(--overlay-surface-strong) 100%);
}

.kiosk-top {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  /* Safe areas: the clock must not sit under a notch in landscape. */
  padding: calc(var(--space-lg) + var(--safe-top))
    calc(var(--space-xl) + var(--safe-right)) var(--space-lg)
    calc(var(--space-xl) + var(--safe-left));
  z-index: var(--z-sticky);
}

.kiosk-brand {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  color: var(--overlay-text);
  font-size: var(--text-lg);
  font-weight: var(--font-semibold);
  letter-spacing: var(--tracking-tight);
}

.kiosk-meta {
  display: flex;
  align-items: center;
  gap: var(--space-lg);
  color: var(--overlay-text);
}

.kiosk-meta .status-line {
  color: var(--overlay-text);
}

.kiosk-meta .status-line.dim {
  opacity: 0.85;
}

.clock {
  font-size: var(--text-xl);
  font-weight: var(--font-semibold);
}

.kiosk-center {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  padding: var(--space-xl);
  z-index: var(--z-raised);
}

/* ---- Prompt ---- */
.kiosk-prompt {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-lg);
  text-align: center;
}

.scan-ring {
  width: 15rem;
  height: 15rem;
  border-radius: var(--radius-full);
  border: 2px dashed var(--overlay-ring);
}

.scan-ring.scanning {
  border-color: var(--overlay-ring-strong);
  animation: pulse 2s var(--ease-out) infinite;
}

@keyframes pulse {
  0%,
  100% {
    transform: scale(1);
    opacity: 0.9;
  }
  50% {
    transform: scale(1.03);
    opacity: 1;
  }
}

.prompt-text {
  color: var(--overlay-text);
  font-size: var(--text-2xl);
  font-weight: var(--font-semibold);
  max-width: 24ch;
}

.prompt-sub {
  color: var(--overlay-text-muted);
  font-size: var(--text-sm);
}

/* ---- Celebration ---- */
.kiosk-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-2xl) var(--space-3xl);
  border-radius: var(--radius-xl);
  background: var(--overlay-surface);
  backdrop-filter: blur(20px);
  border: 1px solid var(--overlay-border);
  color: var(--overlay-text);
  text-align: center;
  max-width: 32rem;
}

.kiosk-card h2 {
  font-size: var(--text-2xl);
}

.kiosk-card p {
  color: var(--overlay-text-muted);
}

.celebration {
  animation: celebrate var(--duration-slow) var(--ease-out);
}

@keyframes celebrate {
  from {
    transform: scale(0.94);
    opacity: 0;
  }
  to {
    transform: scale(1);
    opacity: 1;
  }
}

.celebration-icon {
  color: var(--green-400);
}

.celebration-name {
  font-size: var(--text-4xl);
  letter-spacing: var(--tracking-tight);
}

.celebration-nis {
  font-size: var(--text-lg);
  color: var(--overlay-text-muted) !important;
}

.celebration-note {
  font-size: var(--text-sm);
}

/* ---- Bottom ---- */
.kiosk-bottom {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  /* Safe areas: the actions must clear the home indicator. */
  padding: var(--space-lg) calc(var(--space-xl) + var(--safe-right))
    calc(var(--space-lg) + var(--safe-bottom)) calc(var(--space-xl) + var(--safe-left));
  z-index: var(--z-sticky);
}

.device-name {
  color: var(--overlay-text-muted);
  font-size: var(--text-sm);
}

.kiosk-actions {
  display: flex;
  gap: var(--space-sm);
}

.on-dark {
  color: var(--overlay-text);
}
.on-dark:hover {
  background: var(--overlay-border);
  color: var(--overlay-text);
}

.row {
  display: flex;
  gap: var(--space-sm);
}

/* ---- Mobile kiosk: the phone IS the kiosk, so scale to the small screen ---- */
@media (max-width: 640px) {
  .kiosk-top,
  .kiosk-bottom {
    padding-left: calc(var(--space-md) + var(--safe-left));
    padding-right: calc(var(--space-md) + var(--safe-right));
  }

  .kiosk-brand,
  .clock {
    font-size: var(--text-base);
  }

  /* The ring is the framing guide; at phone size it must not push the prompt
     off screen. Fluid so it scales with width instead of a fixed 15rem. */
  .scan-ring {
    width: clamp(9rem, 55vw, 15rem);
    height: clamp(9rem, 55vw, 15rem);
  }

  .prompt-text {
    font-size: var(--text-xl);
  }

  .kiosk-card {
    padding: var(--space-xl) var(--space-lg);
  }

  .celebration-name {
    font-size: clamp(1.75rem, 8vw, 2.5rem);
  }

  .celebration-icon :deep(svg) {
    width: 36px;
    height: 36px;
  }
}

/* Landscape phones: height is scarce, so tighten the vertical rhythm. */
@media (max-height: 480px) and (orientation: landscape) {
  .scan-ring {
    width: 7rem;
    height: 7rem;
  }
  .kiosk-top,
  .kiosk-bottom {
    padding-top: calc(var(--space-sm) + var(--safe-top));
    padding-bottom: calc(var(--space-sm) + var(--safe-bottom));
  }
}
</style>
