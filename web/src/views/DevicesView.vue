<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { createDevice, deleteDevice, listDevices, revokeDevice } from '../lib/api'
import { ApiError, type Device } from '../lib/types'
import { formatDate } from '../lib/format'
import AppIcon from '../components/AppIcon.vue'
import StatusDot from '../components/StatusDot.vue'

const devices = ref<Device[]>([])
const loading = ref(true)
const error = ref('')

const showForm = ref(false)
const form = ref({ nama: '', lokasi: '' })
const formError = ref('')
const saving = ref(false)

// The freshly minted token, shown once. Never retrievable again.
const newToken = ref<{ token: string; name: string } | null>(null)
const copied = ref(false)

onMounted(load)

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    devices.value = await listDevices()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal memuat perangkat.'
  } finally {
    loading.value = false
  }
}

async function submit(): Promise<void> {
  formError.value = ''
  if (!form.value.nama.trim()) {
    formError.value = 'Nama perangkat wajib diisi.'
    return
  }
  saving.value = true
  try {
    const created = await createDevice({
      nama: form.value.nama.trim(),
      lokasi: form.value.lokasi.trim() || undefined,
    })
    newToken.value = { token: created.token, name: created.device.nama }
    form.value = { nama: '', lokasi: '' }
    showForm.value = false
    await load()
  } catch (e) {
    formError.value = e instanceof ApiError ? e.message : 'Gagal menyimpan.'
  } finally {
    saving.value = false
  }
}

async function copyToken(): Promise<void> {
  if (!newToken.value) return
  try {
    await navigator.clipboard.writeText(newToken.value.token)
    copied.value = true
    setTimeout(() => (copied.value = false), 2000)
  } catch {
    /* clipboard may be blocked; the token is visible for manual copy */
  }
}

async function revoke(d: Device): Promise<void> {
  if (!confirm(`Cabut akses ${d.nama}? Kiosk ini tidak bisa absen lagi.`)) return
  try {
    await revokeDevice(d.id)
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mencabut.'
  }
}

async function remove(d: Device): Promise<void> {
  if (!confirm(`Hapus perangkat ${d.nama}?`)) return
  try {
    await deleteDevice(d.id)
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal menghapus.'
  }
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Perangkat</h1>
        <p class="muted">Kiosk yang boleh mengirim absensi</p>
      </div>
      <button class="btn btn-primary" type="button" @click="showForm = !showForm">
        <AppIcon :name="showForm ? 'x' : 'plus'" :size="16" />
        <span>{{ showForm ? 'Batal' : 'Tambah perangkat' }}</span>
      </button>
    </header>

    <p v-if="error" class="alert alert-error" role="alert">
      <AppIcon name="alert" :size="16" /><span>{{ error }}</span>
    </p>

    <!-- Token shown exactly once -->
    <div v-if="newToken" class="card card-pad token-box">
      <div class="token-head">
        <AppIcon name="shield-check" :size="20" />
        <div>
          <h2>Token untuk {{ newToken.name }}</h2>
          <p class="muted">Salin sekarang. Token ini tidak bisa ditampilkan lagi.</p>
        </div>
      </div>
      <div class="token-row">
        <code class="token">{{ newToken.token }}</code>
        <button class="btn btn-secondary btn-sm" type="button" @click="copyToken">
          <AppIcon name="copy" :size="15" />
          <span>{{ copied ? 'Tersalin' : 'Salin' }}</span>
        </button>
      </div>
      <button class="btn btn-ghost btn-sm" type="button" @click="newToken = null">
        Saya sudah menyimpannya
      </button>
    </div>

    <form v-if="showForm" class="card card-pad create-form" @submit.prevent="submit">
      <div class="field">
        <label class="label" for="dev-nama">Nama perangkat</label>
        <input id="dev-nama" v-model="form.nama" class="input" placeholder="Kiosk Depan" />
      </div>
      <div class="field">
        <label class="label" for="dev-lokasi">Lokasi (opsional)</label>
        <input id="dev-lokasi" v-model="form.lokasi" class="input" placeholder="Lobi utama" />
      </div>
      <div class="form-actions">
        <button class="btn btn-primary" type="submit" :disabled="saving">
          {{ saving ? 'Menyimpan…' : 'Simpan' }}
        </button>
        <span v-if="formError" class="field-error">{{ formError }}</span>
      </div>
    </form>

    <div class="panel">
      <div v-if="loading" class="rows">
        <div v-for="i in 3" :key="i" class="skeleton row-skel" />
      </div>

      <div v-else-if="devices.length === 0" class="empty">
        <AppIcon name="monitor" :size="28" />
        <h3>Belum ada perangkat</h3>
        <p>Tambahkan perangkat kiosk, lalu tempelkan token-nya di halaman kiosk.</p>
        <button class="btn btn-primary" type="button" @click="showForm = true">
          <AppIcon name="plus" :size="16" /><span>Tambah perangkat</span>
        </button>
      </div>

      <div v-else class="table-scroll">
        <table class="table">
        <thead>
          <tr>
            <th scope="col">Nama</th>
            <th scope="col">Lokasi</th>
            <th scope="col">Status</th>
            <th scope="col">Terakhir aktif</th>
            <th scope="col" class="actions"><span class="sr-only">Aksi</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="d in devices" :key="d.id">
            <td>{{ d.nama }}</td>
            <td class="muted">{{ d.lokasi ?? '—' }}</td>
            <td>
              <StatusDot
                :variant="d.revoked ? 'dot-error' : 'dot-success'"
                :label="d.revoked ? 'Dicabut' : 'Aktif'"
              />
            </td>
            <td class="muted tnum">{{ d.last_seen ? formatDate(d.last_seen) : 'Belum pernah' }}</td>
            <td class="actions">
              <button
                v-if="!d.revoked"
                class="btn btn-ghost btn-sm"
                type="button"
                @click="revoke(d)"
              >
                <AppIcon name="x" :size="15" /><span>Cabut</span>
              </button>
              <button class="btn btn-ghost btn-sm danger-text" type="button" @click="remove(d)">
                Hapus
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

.token-box {
  border-color: var(--accent-300);
  background: var(--info-bg);
}

.token-head {
  display: flex;
  gap: var(--space-sm);
  margin-bottom: var(--space-md);
  color: var(--text-link);
}

.token-head h2 {
  font-size: var(--text-base);
  color: var(--text-primary);
}

.token-row {
  display: flex;
  gap: var(--space-sm);
  align-items: center;
  margin-bottom: var(--space-sm);
}

.token {
  flex: 1;
  font-family: ui-monospace, 'SF Mono', Menlo, monospace;
  font-size: var(--text-xs);
  word-break: break-all;
  padding: var(--space-sm);
  background: var(--surface-raised);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-sm);
}

.create-form {
  display: grid;
  grid-template-columns: 1fr 1fr auto;
  gap: var(--space-md);
  align-items: end;
}

.form-actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
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
  display: flex;
  gap: var(--space-xs);
  justify-content: flex-end;
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
  .actions {
    flex-direction: column;
    align-items: stretch;
  }
}
</style>
