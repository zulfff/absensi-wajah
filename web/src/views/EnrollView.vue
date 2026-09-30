<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { activateStudent, enrollCommit, enrollFrame, getStudent, setConsent } from '../lib/api'
import { Camera, cameraErrorMessage } from '../lib/camera'
import { ApiError, type CommitResponse, type Student } from '../lib/types'
import AppIcon from '../components/AppIcon.vue'

const route = useRoute()
const studentId = route.params.id as string

// The guided capture sequence. Each step names a pose; the admin follows along.
const STEPS = [
  { key: 'lurus', label: 'Menghadap lurus', hint: 'Pandang tepat ke kamera' },
  { key: 'kiri', label: 'Menoleh kiri', hint: 'Putar kepala sedikit ke kiri' },
  { key: 'kanan', label: 'Menoleh kanan', hint: 'Putar kepala sedikit ke kanan' },
  { key: 'atas', label: 'Mendongak', hint: 'Angkat dagu sedikit' },
  { key: 'bawah', label: 'Menunduk', hint: 'Tundukkan kepala sedikit' },
  { key: 'senyum', label: 'Tersenyum', hint: 'Ekspresi netral atau senyum' },
  { key: 'lurus2', label: 'Lurus lagi', hint: 'Hadap lurus sekali lagi' },
  { key: 'lurus3', label: 'Lurus terakhir', hint: 'Tahan sebentar, tanpa bergerak' },
] as const

const NEEDED = 7 // backend requires min_frames = 5; capture a few spare

const student = ref<Student | null>(null)
const videoEl = ref<HTMLVideoElement | null>(null)
const camera = new Camera()

const cameraOn = ref(false)
const cameraError = ref('')
const frames = ref<string[]>([])
const lastFeedback = ref<{ accepted: boolean; score: number; guidance: string } | null>(null)
const capturing = ref(false)
const committing = ref(false)
const commitResult = ref<CommitResponse | null>(null)
const error = ref('')
const activating = ref(false)
const activated = ref(false)
const consentBy = ref('')
const grantingConsent = ref(false)

const currentStep = computed(() => STEPS[Math.min(frames.value.length, STEPS.length - 1)])
const progress = computed(() => Math.min(frames.value.length / NEEDED, 1))
const canCommit = computed(() => frames.value.length >= NEEDED && !committing.value && !commitResult.value)

/** Map an API error to copy that tells the admin what to do. */
function describeError(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 403) {
      return 'Pendaftaran ditolak: persetujuan orang tua/wali belum dicatat untuk siswa ini.'
    }
    if (e.status === 413) {
      return 'Frame terlalu besar. Coba turunkan resolusi kamera atau kualitas gambar.'
    }
    return e.message
  }
  return 'Terjadi kesalahan. Coba lagi.'
}

async function grantConsent(): Promise<void> {
  grantingConsent.value = true
  error.value = ''
  try {
    student.value = await setConsent(studentId, consentBy.value)
  } catch (e) {
    error.value = describeError(e)
  } finally {
    grantingConsent.value = false
  }
}

onMounted(async () => {
  try {
    student.value = (await getStudent(studentId)).student
  } catch (e) {
    error.value = e instanceof ApiError ? describeError(e) : 'Siswa tidak ditemukan.'
  }
  await startCamera()
})

onBeforeUnmount(() => camera.stop())

async function startCamera(): Promise<void> {
  cameraError.value = ''
  if (!videoEl.value) return
  try {
    await camera.start(videoEl.value, { width: 640, height: 480 })
    cameraOn.value = true
  } catch (e) {
    cameraError.value = cameraErrorMessage(e)
  }
}

async function capture(): Promise<void> {
  if (capturing.value || commitResult.value) return
  const image = camera.capture(0.8)
  if (!image) return
  capturing.value = true
  try {
    const fb = await enrollFrame(studentId, image, frames.value.length)
    lastFeedback.value = { accepted: fb.accepted, score: fb.score, guidance: fb.guidance }
    // Only keep frames the server accepted. Rejecting here is the whole point:
    // a bad frame must not enter the gallery.
    if (fb.accepted) frames.value.push(image)
  } catch (e) {
    error.value = describeError(e)
  } finally {
    capturing.value = false
  }
}

/** Auto-capture: grab a frame every 350ms until enough are collected. */
let autoTimer: number | null = null
const autoRunning = ref(false)

function toggleAuto(): void {
  if (autoRunning.value) {
    stopAuto()
    return
  }
  autoRunning.value = true
  const tick = async () => {
    if (!autoRunning.value || frames.value.length >= NEEDED || commitResult.value) {
      stopAuto()
      return
    }
    await capture()
    if (autoRunning.value) autoTimer = window.setTimeout(tick, 350)
  }
  tick()
}

function stopAuto(): void {
  autoRunning.value = false
  if (autoTimer !== null) {
    clearTimeout(autoTimer)
    autoTimer = null
  }
}

function reset(): void {
  frames.value = []
  lastFeedback.value = null
  commitResult.value = null
  activated.value = false
  error.value = ''
}

async function commit(): Promise<void> {
  stopAuto()
  committing.value = true
  error.value = ''
  try {
    commitResult.value = await enrollCommit(studentId, frames.value)
  } catch (e) {
    error.value = describeError(e)
  } finally {
    committing.value = false
  }
}

async function activate(): Promise<void> {
  activating.value = true
  error.value = ''
  try {
    await activateStudent(studentId)
    activated.value = true
  } catch (e) {
    error.value = describeError(e)
  } finally {
    activating.value = false
  }
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div class="head-main">
        <router-link class="btn btn-ghost btn-sm" to="/students">
          <AppIcon name="arrow-left" :size="16" /><span>Siswa</span>
        </router-link>
        <div>
          <h1>Daftarkan wajah</h1>
          <p class="muted">{{ student?.nama ?? '…' }} · {{ student?.nis ?? '' }}</p>
        </div>
      </div>
    </header>

    <p v-if="error" class="alert alert-error" role="alert">
      <AppIcon name="alert" :size="16" /><span>{{ error }}</span>
    </p>

    <!-- Consent gate. Enrollment is refused without it, so the page must let
         the admin record it here rather than dead-ending on a 403. -->
    <div v-if="student && !student.consent_granted" class="consent-gate">
      <div class="consent-copy">
        <AppIcon name="shield-check" :size="20" />
        <div>
          <strong>Persetujuan orang tua/wali belum dicatat</strong>
          <p>
            Data wajah adalah data biometrik dan tidak boleh diproses tanpa
            persetujuan. Catat persetujuan tertulis orang tua/wali dulu, lalu
            pendaftaran bisa dilanjutkan.
          </p>
        </div>
      </div>
      <div class="consent-form">
        <div class="field">
          <label class="label" for="consent-by">Dicatat oleh / atas nama</label>
          <input
            id="consent-by"
            v-model="consentBy"
            class="input"
            placeholder="Nama orang tua/wali"
          />
        </div>
        <button
          class="btn btn-primary"
          type="button"
          :disabled="grantingConsent"
          @click="grantConsent"
        >
          <AppIcon name="check" :size="16" />
          <span>{{ grantingConsent ? 'Menyimpan…' : 'Catat persetujuan' }}</span>
        </button>
      </div>
    </div>

    <div class="grid">
      <!-- Camera / preview -->
      <section class="card camera-card">
        <div class="camera-stage">
          <video ref="videoEl" class="video" playsinline muted />

          <!-- Capture progress ring -->
          <div class="progress" role="progressbar" :aria-valuenow="frames.length" :aria-valuemin="0" :aria-valuemax="NEEDED">
            <span class="tnum">{{ frames.length }}/{{ NEEDED }}</span>
          </div>

          <div v-if="!cameraOn && !cameraError" class="overlay">
            <p>Menyalakan kamera…</p>
          </div>

          <div v-if="cameraError" class="overlay overlay-error">
            <AppIcon name="alert" :size="24" />
            <p>{{ cameraError }}</p>
            <button class="btn btn-secondary btn-sm" type="button" @click="startCamera">
              Coba lagi
            </button>
          </div>
        </div>

        <div class="camera-foot">
          <div class="step">
            <span class="step-label">{{ currentStep.label }}</span>
            <span class="step-hint">{{ currentStep.hint }}</span>
          </div>

          <div class="controls">
            <button
              class="btn btn-primary"
              type="button"
              :disabled="!cameraOn || capturing || !!commitResult"
              @click="capture"
            >
              <AppIcon name="camera" :size="16" />
              <span>{{ capturing ? 'Memeriksa…' : 'Ambil frame' }}</span>
            </button>
            <button
              class="btn btn-secondary"
              type="button"
              :disabled="!cameraOn || !!commitResult || frames.length >= NEEDED"
              @click="toggleAuto"
            >
              <span>{{ autoRunning ? 'Hentikan' : 'Auto' }}</span>
            </button>
          </div>
        </div>

        <!-- Progress bar -->
        <div class="bar-track" aria-hidden="true">
          <div class="bar-fill" :style="{ width: `${progress * 100}%` }" />
        </div>
      </section>

      <!-- Quality feedback + frames -->
      <section class="side">
        <div class="card card-pad">
          <h2 class="side-title">Umpan balik kualitas</h2>
          <div v-if="!lastFeedback" class="feedback-idle">
            <AppIcon name="scan-face" :size="22" />
            <p>Ambil frame pertama. Setiap frame diperiksa di server: satu wajah,
              jarak, ketajaman, cahaya, dan pose.</p>
          </div>
          <div v-else class="feedback" :class="lastFeedback.accepted ? 'ok' : 'bad'">
            <span class="status-line">
              <span class="dot" :class="lastFeedback.accepted ? 'dot-success' : 'dot-error'" />
              <strong>{{ lastFeedback.accepted ? 'Frame diterima' : 'Frame ditolak' }}</strong>
            </span>
            <p>{{ lastFeedback.guidance }}</p>
            <p v-if="lastFeedback.accepted" class="score tnum">
              Skor kualitas {{ (lastFeedback.score * 100).toFixed(0) }}%
            </p>
          </div>
        </div>

        <div class="card card-pad">
          <h2 class="side-title">Frame tersimpan ({{ frames.length }})</h2>
          <div v-if="frames.length === 0" class="frames-empty">Belum ada frame.</div>
          <div v-else class="thumbs">
            <img
              v-for="(f, i) in frames"
              :key="i"
              :src="`data:image/jpeg;base64,${f}`"
              :alt="`Frame ${i + 1}`"
              class="thumb"
            />
          </div>
        </div>

        <!-- Commit result -->
        <div v-if="commitResult" class="card card-pad">
          <h2 class="side-title">Hasil pendaftaran</h2>

          <div v-if="commitResult.status === 'accepted'" class="result">
            <p class="alert alert-success">
              <AppIcon name="check-circle" :size="16" />
              <span>
                {{ commitResult.templates_created }} template dibuat
                <template v-if="commitResult.dropped_frames > 0">
                  ({{ commitResult.dropped_frames }} frame dibuang sebagai outlier)
                </template>.
              </span>
            </p>
            <p class="hint">
              Jarak ke siswa terdekat: {{ commitResult.nearest_neighbour_similarity.toFixed(3) }}
              — semakin rendah semakin aman.
            </p>

            <p v-if="activated" class="alert alert-success">
              <AppIcon name="check-circle" :size="16" /><span>Wajah diaktifkan. Siswa sudah bisa absen.</span>
            </p>
            <div v-else class="result-actions">
              <button class="btn btn-primary" type="button" :disabled="activating" @click="activate">
                <AppIcon name="user-check" :size="16" />
                <span>{{ activating ? 'Mengaktifkan…' : 'Aktifkan wajah' }}</span>
              </button>
              <router-link class="btn btn-secondary" to="/students">Selesai</router-link>
            </div>
          </div>

          <div v-else-if="commitResult.status === 'duplicate'" class="result">
            <p class="alert alert-error">
              <AppIcon name="alert" :size="16" />
              <span>
                Diblokir: wajah ini terlalu mirip dengan siswa lain
                ({{ (commitResult.similarity * 100).toFixed(1) }}%). Kemungkinan orang
                yang sama terdaftar dua kali.
              </span>
            </p>
            <button class="btn btn-secondary" type="button" @click="reset">Ulangi capture</button>
          </div>

          <div v-else-if="commitResult.status === 'risky_separation'" class="result">
            <p class="alert alert-warning">
              <AppIcon name="alert" :size="16" />
              <span>
                Berisiko tertukar: terlalu mirip dengan siswa lain
                ({{ (commitResult.similarity * 100).toFixed(1) }}%). Tambah capture
                dengan variasi pose/cahaya, atau review manual.
              </span>
            </p>
            <button class="btn btn-secondary" type="button" @click="reset">Ulangi capture</button>
          </div>

          <div v-else-if="commitResult.status === 'insufficient_frames'" class="result">
            <p class="alert alert-warning">
              <AppIcon name="alert" :size="16" />
              <span>Frame valid kurang ({{ commitResult.kept }}/{{ commitResult.needed }}). Ambil lebih banyak frame.</span>
            </p>
            <button class="btn btn-secondary" type="button" @click="reset">Ulangi capture</button>
          </div>

          <div v-else class="result">
            <p class="alert alert-warning">
              <AppIcon name="alert" :size="16" /><span>Tidak ada frame valid. Ulangi capture.</span>
            </p>
            <button class="btn btn-secondary" type="button" @click="reset">Ulangi capture</button>
          </div>
        </div>

        <div v-if="!commitResult" class="commit-bar">
          <button class="btn btn-primary" type="button" :disabled="!canCommit" @click="commit">
            <AppIcon name="check" :size="16" />
            <span>{{ committing ? 'Menyimpan…' : `Simpan pendaftaran (${frames.length}/${NEEDED})` }}</span>
          </button>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: var(--space-xl);
  /* width + centring come from the global .page rule */
}

.head-main {
  display: flex;
  align-items: center;
  gap: var(--space-md);
}

.page-head h1 {
  font-size: var(--text-2xl);
}

.muted {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

/* Consent gate: an actionable panel, not a dead-end warning. */
.consent-gate {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-lg);
  flex-wrap: wrap;
  padding: var(--card-pad);
  border: 1px solid var(--warning-border);
  border-radius: var(--card-radius);
  background: var(--warning-bg);
}

.consent-copy {
  display: flex;
  gap: var(--space-sm);
  color: var(--text-warning);
  max-width: 42rem;
}

.consent-copy strong {
  display: block;
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.consent-copy p {
  margin-top: var(--space-xs);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.consent-form {
  display: flex;
  align-items: flex-end;
  gap: var(--space-sm);
}

.consent-form .field {
  min-width: 14rem;
}

@media (max-width: 640px) {
  .consent-gate {
    align-items: stretch;
  }
  .consent-form {
    flex-direction: column;
    align-items: stretch;
  }
  .consent-form .field {
    min-width: 0;
  }
}

.grid {
  display: grid;
  grid-template-columns: 1.5fr 1fr;
  gap: var(--space-lg);
  align-items: start;
}

.camera-card {
  overflow: hidden;
}

.camera-stage {
  position: relative;
  aspect-ratio: 4 / 3;
  background: var(--surface-sunken);
  overflow: hidden;
}

.video {
  width: 100%;
  height: 100%;
  object-fit: cover;
  transform: scaleX(-1);
}

.progress {
  position: absolute;
  top: var(--space-sm);
  right: var(--space-sm);
  padding: 0.25rem var(--space-sm);
  border-radius: var(--radius-full);
  background: var(--overlay-surface);
  color: var(--overlay-text);
  font-size: var(--text-sm);
  font-weight: var(--font-medium);
}

.overlay {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-sm);
  background: var(--surface-sunken);
  color: var(--text-secondary);
  font-size: var(--text-sm);
  text-align: center;
  padding: var(--space-lg);
}

.overlay-error {
  color: var(--text-error);
}

.camera-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  padding: var(--space-md) var(--space-lg);
}

.step {
  display: flex;
  flex-direction: column;
}

.step-label {
  font-weight: var(--font-semibold);
  font-size: var(--text-sm);
}

.step-hint {
  font-size: var(--text-xs);
  color: var(--text-secondary);
}

.controls {
  display: flex;
  gap: var(--space-sm);
  flex-wrap: wrap;
}

.bar-track {
  height: 4px;
  background: var(--surface-sunken);
}

.bar-fill {
  height: 100%;
  background: var(--accent-bg);
  transition: width var(--duration-normal) var(--ease-out);
}

.side {
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
}

.side-title {
  font-size: var(--text-sm);
  font-weight: var(--font-semibold);
  margin-bottom: var(--space-sm);
}

.feedback-idle {
  display: flex;
  gap: var(--space-sm);
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.feedback {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  font-size: var(--text-sm);
}

.feedback.ok {
  color: var(--text-primary);
}
.feedback.bad {
  color: var(--text-error);
}

.score {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.frames-empty {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.thumbs {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(3.5rem, 1fr));
  gap: var(--space-xs);
}

.thumb {
  width: 100%;
  aspect-ratio: 1;
  object-fit: cover;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-subtle);
}

.result {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.result-actions {
  display: flex;
  gap: var(--space-sm);
}

.hint {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.commit-bar {
  position: sticky;
  bottom: var(--space-md);
}

/* On mobile the fixed bottom nav (72px) occupies the bottom edge, so the
 * sticky commit bar must clear it or the primary action hides behind the nav. */
@media (max-width: 768px) {
  .commit-bar {
    bottom: calc(4.5rem + var(--safe-bottom));
  }
}

.commit-bar .btn {
  width: 100%;
}

@media (max-width: 900px) {
  .grid {
    grid-template-columns: 1fr;
  }
}
</style>
