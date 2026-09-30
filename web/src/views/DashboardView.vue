<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { listAttendance, monitoringSummary } from '../lib/api'
import { ApiError, type AttendanceRow, type MonitoringSummary } from '../lib/types'
import { formatTime, percent, todayISO } from '../lib/format'
import AppIcon from '../components/AppIcon.vue'
import Sparkline from '../components/Sparkline.vue'

const summary = ref<MonitoringSummary | null>(null)
const recent = ref<AttendanceRow[]>([])
const todayRows = ref<AttendanceRow[]>([])
const loading = ref(true)
const error = ref('')

const accepted = computed(() => summary.value?.outcomes.find((o) => o.outcome === 'accepted')?.count ?? 0)
const cooldown = computed(() => summary.value?.outcomes.find((o) => o.outcome === 'cooldown_shown')?.count ?? 0)
const rejected = computed(() =>
  (summary.value?.outcomes ?? [])
    .filter((o) => o.outcome.startsWith('rejected'))
    .reduce((sum, o) => sum + o.count, 0),
)
const totalAttempts = computed(() => (summary.value?.outcomes ?? []).reduce((s, o) => s + o.count, 0))
const rejectRatio = computed(() => (totalAttempts.value === 0 ? 0 : rejected.value / totalAttempts.value))

// Sparkline of today's check-ins bucketed by hour (06:00-18:00), so the hero
// shows the arrival wave rather than a static number.
//
// Buckets use the admin device's local wall clock. The attendance list is
// already date-filtered in the school's reporting timezone server-side; on the
// intended deployment (a school admin on the same local network) the two agree.
const hourly = computed(() => {
  const buckets = new Array(13).fill(0) // 06..18
  for (const row of todayRows.value) {
    const h = new Date(row.timestamp).getHours()
    if (h >= 6 && h <= 18) buckets[h - 6] += 1
  }
  return buckets
})

onMounted(load)

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const [s, a] = await Promise.all([monitoringSummary(), listAttendance({ tanggal: todayISO() })])
    summary.value = s
    todayRows.value = a
    recent.value = a.slice(0, 8)
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal memuat data.'
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Ringkasan</h1>
        <p class="muted">Aktivitas kiosk 24 jam terakhir</p>
      </div>
      <button class="btn btn-ghost btn-sm" type="button" :disabled="loading" @click="load">
        <AppIcon name="rotate-ccw" :size="16" />
        <span>Muat ulang</span>
      </button>
    </header>

    <p v-if="error" class="alert alert-error" role="alert">
      <AppIcon name="alert" :size="16" /><span>{{ error }}</span>
    </p>

    <!-- Hero: the one thing that dominates. Flush on canvas, not in a card. -->
    <section class="hero" aria-label="Absensi hari ini">
      <div class="hero-main">
        <span class="hero-label">Absensi tercatat (24 jam terakhir)</span>
        <div class="hero-figure">
          <span class="hero-value tnum">{{ loading ? '—' : accepted }}</span>
          <span class="hero-unit">check-in</span>
        </div>
        <Sparkline :values="hourly" :width="320" :height="44" label="Distribusi check-in per jam, 06:00 sampai 18:00" />
        <span class="hero-range">06:00 — 18:00</span>
      </div>

      <!-- Support strip: compact inline, not three more equal cards. -->
      <dl class="hero-stats">
        <div class="stat">
          <dt>Galeri aktif</dt>
          <dd class="tnum">{{ summary?.gallery_students ?? '—' }}</dd>
          <span class="stat-note">siswa terdaf&shy;tar</span>
        </div>
        <div class="stat">
          <dt>Sudah absen</dt>
          <dd class="tnum">{{ cooldown }}</dd>
          <span class="stat-note">percobaan berulang</span>
        </div>
        <div class="stat">
          <dt>Rasio ditolak</dt>
          <dd class="tnum">{{ percent(rejectRatio) }}</dd>
          <span class="stat-note">{{ rejected }} dari {{ totalAttempts }}</span>
        </div>
      </dl>
    </section>

    <!-- Work queue: the table is the product, wrapped in the same panel the
         other list screens use so the rhythm is consistent across pages. -->
    <section class="queue" aria-label="Absensi terbaru">
      <div class="panel">
        <div class="panel-head">
          <h2>Absensi terbaru</h2>
          <router-link class="link-quiet" to="/attendance">Lihat semua</router-link>
        </div>

        <div v-if="loading" class="queue-body">
          <div v-for="i in 4" :key="i" class="skeleton row-skel" />
        </div>

        <div v-else-if="recent.length === 0" class="empty">
          <AppIcon name="calendar-check" :size="26" />
          <h3>Belum ada absensi hari ini</h3>
          <p>Check-in muncul setelah siswa menghadap kamera kiosk yang sudah terhubung.</p>
          <router-link class="btn btn-secondary btn-sm" to="/students">Kelola siswa</router-link>
        </div>

        <div v-else class="table-scroll">
          <table class="table">
          <thead>
            <tr>
              <th scope="col">Siswa</th>
              <th scope="col">NIS</th>
              <th scope="col">Kemiripan</th>
              <th scope="col" class="num">Margin</th>
              <th scope="col">Waktu</th>
              <th scope="col">Status</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in recent" :key="row.id">
              <td data-label="Siswa" class="cell-name">
                <span class="avatar" aria-hidden="true">{{ row.nama.slice(0, 1) }}</span>
                {{ row.nama }}
              </td>
              <td data-label="NIS" class="muted tnum">{{ row.nis }}</td>
              <td data-label="Kemiripan">
                <!-- Proportion bar: relative confidence is easier to scan than a
                     bare number, and it makes a weak match visible at a glance. -->
                <span class="score-cell">
                  <span class="score-bar" :style="{ '--pct': `${Math.round(row.similarity * 100)}%` }" />
                  <span class="tnum score-num">{{ percent(row.similarity) }}</span>
                </span>
              </td>
              <td data-label="Margin" class="num tnum">{{ row.margin.toFixed(2) }}</td>
              <td data-label="Waktu" class="tnum">{{ formatTime(row.timestamp) }}</td>
              <td data-label="Status">
                <span class="status-line">
                  <span class="dot" :class="row.status === 'present' ? 'dot-success' : 'dot-warning'" />
                  {{ row.status === 'present' ? 'Hadir' : row.status }}
                </span>
              </td>
            </tr>
          </tbody>
        </table>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: var(--space-xl);
  /* width + centring come from the global .page rule */
}

.page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-md);
}

.page-head h1 {
  font-size: var(--text-2xl);
}

.muted {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

/* ---- Hero ---- */

.hero {
  display: grid;
  grid-template-columns: minmax(0, 20rem) 1fr;
  gap: var(--space-2xl);
  align-items: end;
  padding-bottom: var(--space-lg);
  border-bottom: 1px solid var(--border-default);
}

.hero-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.hero-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.hero-figure {
  display: flex;
  align-items: baseline;
  gap: var(--space-sm);
}

.hero-value {
  font-size: var(--text-6xl);
  font-weight: var(--font-bold);
  letter-spacing: -0.03em;
  line-height: 0.95;
  color: var(--accent-bg);
}

.hero-unit {
  font-size: var(--text-lg);
  color: var(--text-tertiary);
}

.hero-range {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

/* Support stats: an inline definition list, one row, hairline separators. */
.hero-stats {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-lg);
}

.stat {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  padding-left: var(--space-md);
  border-left: 1px solid var(--border-subtle);
}

.stat dt {
  font-size: var(--text-xs);
  color: var(--text-secondary);
}

.stat dd {
  font-size: var(--text-2xl);
  font-weight: var(--font-semibold);
  letter-spacing: -0.02em;
  line-height: 1.1;
}

.stat-note {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

/* ---- Queue ---- */

.queue {
  display: flex;
  flex-direction: column;
}

.link-quiet {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}
.link-quiet:hover {
  color: var(--text-link);
}

/* Keep the quiet look but give it a full touch target on touch devices. */
@media (pointer: coarse) {
  .link-quiet {
    display: inline-flex;
    align-items: center;
    min-height: var(--touch-target);
  }
}

.queue-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
  padding: var(--space-md) var(--panel-pad-x);
}

.row-skel {
  height: var(--row-height);
}

/* Row context: a monogram avatar so the table is not a bare grid. */
.cell-name {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  font-weight: var(--font-medium);
}

.avatar {
  display: grid;
  place-items: center;
  width: 1.5rem;
  height: 1.5rem;
  flex: none;
  border-radius: var(--radius-full);
  background: var(--surface-sunken);
  color: var(--text-secondary);
  font-size: var(--text-xs);
  font-weight: var(--font-semibold);
}

.score-cell {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  min-width: 8rem;
}

.score-bar {
  position: relative;
  flex: 1;
  height: 4px;
  border-radius: var(--radius-full);
  background: var(--surface-sunken);
  overflow: hidden;
}

.score-bar::after {
  content: '';
  position: absolute;
  inset: 0 auto 0 0;
  width: var(--pct);
  background: var(--accent-bg);
  border-radius: var(--radius-full);
}

.score-num {
  font-size: var(--text-xs);
  color: var(--text-secondary);
  min-width: 3rem;
  text-align: right;
}

@media (max-width: 900px) {
  .hero {
    grid-template-columns: 1fr;
    gap: var(--space-lg);
    align-items: start;
  }
  .hero-stats {
    gap: var(--space-md);
  }
}

@media (max-width: 560px) {
  .hero-stats {
    grid-template-columns: 1fr;
  }
  .stat {
    padding-left: 0;
    border-left: 0;
    border-top: 1px solid var(--border-subtle);
    padding-top: var(--space-sm);
  }
}
</style>
