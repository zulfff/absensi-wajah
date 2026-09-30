<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import {
  activateStudent,
  createStudent,
  deleteFace,
  deleteStudent,
  listStudents,
  setConsent,
} from '../lib/api'
import { ApiError, type Student, type StudentListItem } from '../lib/types'
import AppIcon from '../components/AppIcon.vue'
import StatusDot from '../components/StatusDot.vue'

const students = ref<StudentListItem[]>([])
const templateCounts = ref<Record<string, number>>({})
const loading = ref(true)
const error = ref('')
const query = ref('')

// Create form
const showForm = ref(false)
const form = ref({ nis: '', nama: '' })
const formError = ref('')
const saving = ref(false)

const busyId = ref<string | null>(null)

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!q) return students.value
  return students.value.filter((s) => s.nama.toLowerCase().includes(q) || s.nis.toLowerCase().includes(q))
})

const hasStudents = computed(() => students.value.length > 0)
const filteredEmpty = computed(() => hasStudents.value && filtered.value.length === 0)

onMounted(load)

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    // The list endpoint now carries the template counts, so this is a single
    // request instead of one per student.
    const rows = await listStudents()
    students.value = rows
    templateCounts.value = Object.fromEntries(
      rows.map((s) => [s.id, s.active_template_count]),
    )
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal memuat daftar siswa.'
  } finally {
    loading.value = false
  }
}

async function submit(): Promise<void> {
  formError.value = ''
  if (!form.value.nis.trim() || !form.value.nama.trim()) {
    formError.value = 'NIS dan nama wajib diisi.'
    return
  }
  saving.value = true
  try {
    await createStudent({ nis: form.value.nis.trim(), nama: form.value.nama.trim() })
    form.value = { nis: '', nama: '' }
    showForm.value = false
    await load()
  } catch (e) {
    formError.value = e instanceof ApiError ? e.message : 'Gagal menyimpan.'
  } finally {
    saving.value = false
  }
}

async function grantConsent(student: Student): Promise<void> {
  busyId.value = student.id
  try {
    await setConsent(student.id)
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mencatat persetujuan.'
  } finally {
    busyId.value = null
  }
}

async function toggleActive(student: Student): Promise<void> {
  busyId.value = student.id
  try {
    if ((templateCounts.value[student.id] ?? 0) > 0) {
      await activateStudent(student.id)
    }
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mengaktifkan.'
  } finally {
    busyId.value = null
  }
}

async function removeFace(student: Student): Promise<void> {
  if (!confirm(`Hapus data wajah ${student.nama}? Tindakan ini tidak bisa dibatalkan.`)) return
  busyId.value = student.id
  try {
    await deleteFace(student.id)
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal menghapus data wajah.'
  } finally {
    busyId.value = null
  }
}

async function removeStudent(student: Student): Promise<void> {
  if (!confirm(`Hapus siswa ${student.nama}? Semua data wajah dan absensinya ikut terhapus.`)) return
  busyId.value = student.id
  try {
    await deleteStudent(student.id)
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal menghapus siswa.'
  } finally {
    busyId.value = null
  }
}

function enrollmentState(id: string): { dot: string; label: string } {
  const n = templateCounts.value[id] ?? 0
  if (n === 0) return { dot: 'dot-muted', label: 'Belum ada wajah' }
  return { dot: 'dot-success', label: `${n} template` }
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Siswa</h1>
        <p class="muted">{{ students.length }} siswa terdaftar</p>
      </div>
      <button class="btn btn-primary" type="button" @click="showForm = !showForm">
        <AppIcon :name="showForm ? 'x' : 'plus'" :size="16" />
        <span>{{ showForm ? 'Batal' : 'Tambah siswa' }}</span>
      </button>
    </header>

    <p v-if="error" class="alert alert-error" role="alert">
      <AppIcon name="alert" :size="16" /><span>{{ error }}</span>
    </p>

    <form v-if="showForm" class="card card-pad create-form" @submit.prevent="submit">
      <div class="field">
        <label class="label" for="nis">NIS</label>
        <input id="nis" v-model="form.nis" class="input" :class="{ 'has-error': formError }" />
      </div>
      <div class="field">
        <label class="label" for="nama">Nama lengkap</label>
        <input id="nama" v-model="form.nama" class="input" :class="{ 'has-error': formError }" />
      </div>
      <div class="form-actions">
        <button class="btn btn-primary" type="submit" :disabled="saving">
          {{ saving ? 'Menyimpan…' : 'Simpan' }}
        </button>
        <span v-if="formError" class="field-error">{{ formError }}</span>
      </div>
    </form>

    <div class="panel">
      <div class="toolbar">
        <label class="sr-only" for="search">Cari siswa</label>
        <input
          id="search"
          v-model="query"
          class="input search"
          type="search"
          placeholder="Cari nama atau NIS…"
        />
        <button v-if="query" class="btn btn-ghost btn-sm" type="button" @click="query = ''">
          Hapus filter
        </button>
      </div>

      <!-- Loading -->
      <div v-if="loading" class="rows">
        <div v-for="i in 5" :key="i" class="skeleton row-skel" />
      </div>

      <!-- First-run empty -->
      <div v-else-if="!hasStudents" class="empty">
        <AppIcon name="users" :size="28" />
        <h3>Belum ada siswa</h3>
        <p>Tambahkan siswa terlebih dahulu, lalu daftarkan wajahnya lewat halaman pendaftaran.</p>
        <button class="btn btn-primary" type="button" @click="showForm = true">
          <AppIcon name="plus" :size="16" /><span>Tambah siswa</span>
        </button>
      </div>

      <!-- Filtered empty -->
      <div v-else-if="filteredEmpty" class="empty">
        <AppIcon name="users" :size="28" />
        <h3>Tidak ada hasil untuk “{{ query }}”</h3>
        <p>Filter ini tidak cocok dengan siswa mana pun.</p>
        <button class="btn btn-secondary" type="button" @click="query = ''">Hapus filter</button>
      </div>

      <div v-else class="table-scroll">
        <table class="table">
        <thead>
          <tr>
            <th scope="col">Nama</th>
            <th scope="col">NIS</th>
            <th scope="col">Persetujuan</th>
            <th scope="col">Wajah</th>
            <th scope="col">Status</th>
            <th scope="col" class="actions"><span class="sr-only">Aksi</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in filtered" :key="s.id">
            <td>{{ s.nama }}</td>
            <td class="tnum muted">{{ s.nis }}</td>
            <td>
              <StatusDot
                v-if="s.consent_granted"
                variant="dot-success"
                label="Tercatat"
              />
              <button
                v-else
                class="btn btn-ghost btn-sm consent-btn"
                type="button"
                :disabled="busyId === s.id"
                @click="grantConsent(s)"
              >
                <AppIcon name="shield-check" :size="15" />
                <span>Catat persetujuan</span>
              </button>
            </td>
            <td>
              <StatusDot
                :variant="enrollmentState(s.id).dot"
                :label="enrollmentState(s.id).label"
              />
            </td>
            <td>
              <StatusDot
                :variant="s.status === 'active' ? 'dot-success' : 'dot-muted'"
                :label="s.status === 'active' ? 'Aktif' : 'Nonaktif'"
              />
            </td>
            <td class="actions">
              <router-link
                class="btn btn-ghost btn-sm"
                :to="`/students/${s.id}/enroll`"
              >
                <AppIcon name="camera" :size="15" />
                <span>{{ (templateCounts[s.id] ?? 0) > 0 ? 'Daftar ulang' : 'Daftarkan wajah' }}</span>
              </router-link>
              <button
                v-if="(templateCounts[s.id] ?? 0) > 0"
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busyId === s.id"
                @click="toggleActive(s)"
              >
                <AppIcon name="user-check" :size="15" />
                <span>Aktifkan</span>
              </button>
              <button
                v-if="(templateCounts[s.id] ?? 0) > 0"
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busyId === s.id"
                @click="removeFace(s)"
              >
                <AppIcon name="trash" :size="15" />
                <span>Hapus wajah</span>
              </button>
              <button
                class="btn btn-ghost btn-sm danger-text"
                type="button"
                :disabled="busyId === s.id"
                @click="removeStudent(s)"
              >
                <span>Hapus</span>
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
  gap: var(--space-lg);
  max-width: 72rem;
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

.create-form {
  display: grid;
  grid-template-columns: 1fr 2fr auto;
  gap: var(--space-md);
  align-items: end;
}

.form-actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: var(--space-md) var(--space-lg);
  border-bottom: 1px solid var(--border-subtle);
}

.search {
  max-width: 22rem;
}

.rows {
  padding: var(--space-md) var(--space-lg);
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.row-skel {
  height: var(--row-height);
}

.actions {
  display: flex;
  gap: var(--space-xs);
  justify-content: flex-end;
  white-space: nowrap;
}

/* The consent action is the one thing that unblocks enrollment, so it gets a
 * subtle attention tint rather than reading as just another ghost button. */
.consent-btn {
  color: var(--text-warning);
  white-space: nowrap;
}
.consent-btn:hover {
  background: var(--warning-bg);
  color: var(--text-warning);
}

.danger-text {
  color: var(--text-error);
}
.danger-text:hover {
  background: var(--error-bg);
}

@media (max-width: 800px) {
  .create-form {
    grid-template-columns: 1fr;
  }
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
