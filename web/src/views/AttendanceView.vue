<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { correctAttendance, listAttendance, listStudents, markAttendanceManual } from '../lib/api'
import { ApiError, type AttendanceRow, type StudentListItem } from '../lib/types'
import { formatTime, percent, todayISO } from '../lib/format'
import AppIcon from '../components/AppIcon.vue'
import StatusDot from '../components/StatusDot.vue'

const rows = ref<AttendanceRow[]>([])
const loading = ref(true)
const error = ref('')
const date = ref(todayISO())
const filterNama = ref('')
const busyId = ref<string | null>(null)

// Manual attendance (teacher fallback when the camera cannot recognise).
const students = ref<StudentListItem[]>([])
const manualOpen = ref(false)
const manualQuery = ref('')
const manualId = ref('')
const manualNote = ref('')
const manualBusy = ref(false)
const notice = ref('')

const manualMatches = computed(() => {
  const q = manualQuery.value.trim().toLowerCase()
  if (!q) return students.value.slice(0, 20)
  return students.value
    .filter((s) => s.nama.toLowerCase().includes(q) || s.nis.toLowerCase().includes(q))
    .slice(0, 20)
})

const filtered = computed(() => {
  const q = filterNama.value.trim().toLowerCase()
  if (!q) return rows.value
  return rows.value.filter((r) => r.nama.toLowerCase().includes(q) || r.nis.toLowerCase().includes(q))
})

const hasRows = computed(() => rows.value.length > 0)
const filteredEmpty = computed(() => hasRows.value && filtered.value.length === 0)
const filtersActive = computed(() => filterNama.value.trim() !== '')

onMounted(() => {
  void load()
  void loadStudents()
})

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    rows.value = await listAttendance({ tanggal: date.value })
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal memuat absensi.'
  } finally {
    loading.value = false
  }
}

/** Load the roster once, for the manual-mark picker. */
async function loadStudents(): Promise<void> {
  try {
    students.value = await listStudents(500, 0)
  } catch {
    /* the picker is optional; the list view still works without it */
  }
}

async function submitManual(): Promise<void> {
  if (!manualId.value) return
  manualBusy.value = true
  error.value = ''
  notice.value = ''
  try {
    await markAttendanceManual(manualId.value, manualNote.value)
    const s = students.value.find((x) => x.id === manualId.value)
    notice.value = `${s?.nama ?? 'Siswa'} ditandai hadir (manual).`
    manualId.value = ''
    manualQuery.value = ''
    manualNote.value = ''
    manualOpen.value = false
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal menandai hadir manual.'
  } finally {
    manualBusy.value = false
  }
}

async function correct(row: AttendanceRow): Promise<void> {
  if (!confirm(`Tandai absensi ${row.nama} sebagai dikoreksi (dibatalkan)?`)) return
  busyId.value = row.id
  try {
    await correctAttendance(row.id, 'dikoreksi dari dashboard')
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mengoreksi.'
  } finally {
    busyId.value = null
  }
}

/** Escape one CSV cell: prevent spreadsheet formula injection AND quote safely.
 *
 * A cell beginning with `= + - @` (or a leading tab/CR/LF) is executed as a
 * formula by Excel/Sheets, so prefix it with a single quote. Then wrap in quotes
 * and double any embedded quotes (RFC 4180). */
function csvCell(value: unknown): string {
  let s = String(value)
  if (/^[=+\-@\t\r\n]/.test(s)) s = `'${s}`
  return `"${s.replace(/"/g, '""')}"`
}

/** Export exactly the rows currently shown, and say so in the button. */
function exportCsv(): void {
  const header = ['nama', 'nis', 'waktu', 'status', 'kemiripan', 'margin', 'liveness']
  const lines = filtered.value.map((r) =>
    [r.nama, r.nis, r.timestamp, r.status, r.similarity.toFixed(4), r.margin.toFixed(4), r.liveness_score.toFixed(4)]
      .map(csvCell)
      .join(','),
  )
  const csv = [header.join(','), ...lines].join('\n')
  const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `absensi-${date.value}.csv`
  a.click()
  URL.revokeObjectURL(url)
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Absensi</h1>
        <p class="muted">Catatan kehadiran per tanggal</p>
      </div>
      <button class="btn btn-secondary btn-sm" type="button" :disabled="filtered.length === 0" @click="exportCsv">
        <AppIcon name="copy" :size="15" />
        <span>Ekspor {{ filtered.length }} baris</span>
      </button>
    </header>

    <p v-if="error" class="alert alert-error" role="alert">
      <AppIcon name="alert" :size="16" /><span>{{ error }}</span>
    </p>
    <p v-if="notice" class="alert alert-success" role="status">
      <AppIcon name="check-circle" :size="16" /><span>{{ notice }}</span>
    </p>

    <!-- Teacher fallback: when the kiosk camera cannot recognise a student
         (broken camera, glasses, lighting), staff record presence by hand. -->
    <div class="card card-pad manual-card">
      <div class="manual-head">
        <div>
          <strong>Absen manual</strong>
          <p class="muted">Untuk kasus kamera tidak mengenali siswa. Tercatat sebagai <em>manual</em> di audit.</p>
        </div>
        <button class="btn btn-secondary btn-sm" type="button" @click="manualOpen = !manualOpen">
          <AppIcon :name="manualOpen ? 'x' : 'user-check'" :size="15" />
          <span>{{ manualOpen ? 'Batal' : 'Tandai hadir' }}</span>
        </button>
      </div>

      <div v-if="manualOpen" class="manual-form">
        <div class="field">
          <label class="label" for="manual-search">Cari siswa</label>
          <input
            id="manual-search"
            v-model="manualQuery"
            class="input"
            type="search"
            placeholder="Nama atau NIS…"
          />
        </div>
        <div class="field">
          <label class="label" for="manual-pick">Siswa</label>
          <select id="manual-pick" v-model="manualId" class="select">
            <option value="">— pilih siswa —</option>
            <option v-for="s in manualMatches" :key="s.id" :value="s.id">
              {{ s.nama }} · {{ s.nis }}
            </option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="manual-note">Catatan (opsional)</label>
          <input id="manual-note" v-model="manualNote" class="input" placeholder="mis. wajah belum terdaftar" />
        </div>
        <button class="btn btn-primary" type="button" :disabled="!manualId || manualBusy" @click="submitManual">
          <AppIcon name="check" :size="16" />
          <span>{{ manualBusy ? 'Menyimpan…' : 'Tandai hadir' }}</span>
        </button>
      </div>
    </div>

    <div class="panel">
      <div class="panel-toolbar">
        <div class="field date-field">
          <label class="label" for="tanggal">Tanggal</label>
          <input id="tanggal" v-model="date" class="input" type="date" @change="load" />
        </div>
        <div class="field search-field">
          <label class="label" for="af">Cari siswa</label>
          <input id="af" v-model="filterNama" class="input" type="search" placeholder="Nama atau NIS…" />
        </div>
        <button
          v-if="filtersActive"
          class="btn btn-ghost btn-sm reset-btn"
          type="button"
          @click="filterNama = ''"
        >
          Hapus filter
        </button>
      </div>

      <div v-if="loading" class="rows">
        <div v-for="i in 5" :key="i" class="skeleton row-skel" />
      </div>

      <div v-else-if="!hasRows" class="empty">
        <AppIcon name="calendar-check" :size="28" />
        <h3>Tidak ada absensi pada {{ date }}</h3>
        <p>Pilih tanggal lain, atau check-in akan muncul saat siswa menghadap kamera kiosk.</p>
      </div>

      <div v-else-if="filteredEmpty" class="empty">
        <AppIcon name="calendar-check" :size="28" />
        <h3>Tidak ada hasil untuk “{{ filterNama }}”</h3>
        <p>{{ hasRows ? `${rows.length} absensi pada tanggal ini, tapi tidak ada yang cocok.` : '' }}</p>
        <button class="btn btn-secondary" type="button" @click="filterNama = ''">Hapus filter</button>
      </div>

      <div v-else class="table-scroll">
        <table class="table">
        <thead>
          <tr>
            <th scope="col">Siswa</th>
            <th scope="col">NIS</th>
            <th scope="col" class="num">Kemiripan</th>
            <th scope="col" class="num">Liveness</th>
            <th scope="col">Waktu</th>
            <th scope="col">Status</th>
            <th scope="col" class="actions"><span class="sr-only">Aksi</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in filtered" :key="r.id">
            <td data-label="Siswa">{{ r.nama }}</td>
            <td data-label="NIS" class="tnum muted">{{ r.nis }}</td>
            <td data-label="Kemiripan" class="num tnum">{{ percent(r.similarity) }}</td>
            <td data-label="Liveness" class="num tnum">{{ percent(r.liveness_score) }}</td>
            <td data-label="Waktu" class="tnum">{{ formatTime(r.timestamp) }}</td>
            <td data-label="Status">
              <StatusDot
                :variant="r.status === 'present' ? 'dot-success' : r.status === 'corrected' ? 'dot-error' : 'dot-warning'"
                :label="r.status === 'present' ? 'Hadir' : r.status === 'corrected' ? 'Dikoreksi' : r.status"
              />
            </td>
            <td class="actions">
              <button
                v-if="r.status === 'present'"
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busyId === r.id"
                @click="correct(r)"
              >
                Koreksi
              </button>
            </td>
          </tr>
        </tbody>
        </table>
      </div>
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

/* Manual attendance fallback panel. */
.manual-card {
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
}

.manual-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-md);
}

.manual-head p {
  margin-top: var(--space-xs);
  max-width: 44ch;
}

.manual-head em {
  font-style: normal;
  font-weight: var(--font-medium);
  color: var(--text-primary);
}

.manual-form {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr auto;
  gap: var(--space-md);
  align-items: end;
  padding-top: var(--space-md);
  border-top: 1px solid var(--border-subtle);
}

@media (max-width: 800px) {
  .manual-head {
    flex-direction: column;
    align-items: stretch;
  }
  .manual-form {
    grid-template-columns: 1fr;
  }
}

.toolbar {
  display: flex;
  align-items: flex-end;
  gap: var(--space-md);
  padding: var(--space-md) var(--space-lg);
  border-bottom: 1px solid var(--border-subtle);
  flex-wrap: wrap;
}

.date-field {
  max-width: 12rem;
}

.search-field {
  max-width: 20rem;
  flex: 1;
}

.reset-btn {
  margin-bottom: 0.15rem;
}

.rows {
  padding: var(--space-md) var(--panel-pad-x);
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.row-skel {
  height: var(--row-height);
}

.actions {
  text-align: right;
}

@media (max-width: 640px) {
  .page-head {
    flex-direction: column;
    align-items: stretch;
  }
  .page-head > .btn,
  .page-head > a.btn {
    width: 100%;
  }
  .toolbar {
    gap: var(--space-sm);
  }
  .actions {
    flex-direction: column;
    align-items: stretch;
  }
}
</style>
