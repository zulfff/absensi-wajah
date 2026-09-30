<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { createUser, deleteUser, listUsers, updateUser } from '../lib/api'
import { ApiError, type User } from '../lib/types'
import { formatDate } from '../lib/format'
import AppIcon from '../components/AppIcon.vue'
import StatusDot from '../components/StatusDot.vue'

const users = ref<User[]>([])
const loading = ref(true)
const error = ref('')
const notice = ref('')
const busyId = ref<string | null>(null)

// Create form
const showForm = ref(false)
const form = ref({ username: '', password: '', role: 'guru' as 'admin' | 'guru' })
const formError = ref('')
const saving = ref(false)

// Inline password reset
const resettingId = ref<string | null>(null)
const newPassword = ref('')

onMounted(load)

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    users.value = await listUsers()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal memuat akun.'
  } finally {
    loading.value = false
  }
}

async function submit(): Promise<void> {
  formError.value = ''
  if (!form.value.username.trim()) {
    formError.value = 'Nama pengguna wajib diisi.'
    return
  }
  if (form.value.password.length < 8) {
    formError.value = 'Kata sandi minimal 8 karakter.'
    return
  }
  saving.value = true
  try {
    await createUser({
      username: form.value.username.trim(),
      password: form.value.password,
      role: form.value.role,
    })
    form.value = { username: '', password: '', role: 'guru' }
    showForm.value = false
    await load()
  } catch (e) {
    formError.value = e instanceof ApiError ? e.message : 'Gagal menyimpan.'
  } finally {
    saving.value = false
  }
}

async function toggleRole(u: User): Promise<void> {
  const next = u.role === 'admin' ? 'guru' : 'admin'
  if (!confirm(`Ubah peran ${u.username} menjadi ${next}?`)) return
  busyId.value = u.id
  error.value = ''
  try {
    await updateUser(u.id, { role: next })
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mengubah peran.'
  } finally {
    busyId.value = null
  }
}

async function toggleDisabled(u: User): Promise<void> {
  const verb = u.disabled ? 'aktifkan' : 'nonaktifkan'
  if (!confirm(`Yakin ${verb} akun ${u.username}?`)) return
  busyId.value = u.id
  error.value = ''
  try {
    await updateUser(u.id, { disabled: !u.disabled })
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mengubah status.'
  } finally {
    busyId.value = null
  }
}

function startReset(u: User): void {
  resettingId.value = u.id
  newPassword.value = ''
  notice.value = ''
}

async function saveReset(u: User): Promise<void> {
  if (newPassword.value.length < 8) {
    error.value = 'Kata sandi minimal 8 karakter.'
    return
  }
  busyId.value = u.id
  error.value = ''
  try {
    await updateUser(u.id, { password: newPassword.value })
    notice.value = `Kata sandi ${u.username} diperbarui.`
    resettingId.value = null
    newPassword.value = ''
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal mengubah kata sandi.'
  } finally {
    busyId.value = null
  }
}

async function remove(u: User): Promise<void> {
  if (!confirm(`Hapus akun ${u.username}? Tindakan ini tidak bisa dibatalkan.`)) return
  busyId.value = u.id
  error.value = ''
  try {
    await deleteUser(u.id)
    await load()
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal menghapus akun.'
  } finally {
    busyId.value = null
  }
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Akun</h1>
        <p class="muted">Admin dan guru yang bisa masuk dashboard</p>
      </div>
      <button class="btn btn-primary" type="button" @click="showForm = !showForm">
        <AppIcon :name="showForm ? 'x' : 'plus'" :size="16" />
        <span>{{ showForm ? 'Batal' : 'Tambah akun' }}</span>
      </button>
    </header>

    <p v-if="error" class="alert alert-error" role="alert">
      <AppIcon name="alert" :size="16" /><span>{{ error }}</span>
    </p>
    <p v-if="notice" class="alert alert-success" role="status">
      <AppIcon name="check-circle" :size="16" /><span>{{ notice }}</span>
    </p>

    <form v-if="showForm" class="card card-pad create-form" @submit.prevent="submit">
      <div class="field">
        <label class="label" for="u-name">Nama pengguna</label>
        <input id="u-name" v-model="form.username" class="input" autocomplete="off" />
      </div>
      <div class="field">
        <label class="label" for="u-pass">Kata sandi</label>
        <input
          id="u-pass"
          v-model="form.password"
          class="input"
          type="password"
          autocomplete="new-password"
          placeholder="Minimal 8 karakter"
        />
      </div>
      <div class="field">
        <label class="label" for="u-role">Peran</label>
        <select id="u-role" v-model="form.role" class="select">
          <option value="guru">Guru — hanya lihat data</option>
          <option value="admin">Admin — kelola semua</option>
        </select>
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

      <div v-else-if="users.length === 0" class="empty">
        <AppIcon name="users" :size="26" />
        <h3>Belum ada akun</h3>
        <p>Tambahkan akun guru atau admin untuk mengakses dashboard.</p>
        <button class="btn btn-primary" type="button" @click="showForm = true">
          <AppIcon name="plus" :size="16" /><span>Tambah akun</span>
        </button>
      </div>

      <div v-else class="table-scroll">
        <table class="table">
          <thead>
            <tr>
              <th scope="col">Nama pengguna</th>
              <th scope="col">Peran</th>
              <th scope="col">Status</th>
              <th scope="col">Dibuat</th>
              <th scope="col"><span class="sr-only">Aksi</span></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="u in users" :key="u.id">
              <td data-label="Nama pengguna" class="cell-name">{{ u.username }}</td>
              <td data-label="Peran">
                <button
                  class="btn btn-ghost btn-sm role-btn"
                  type="button"
                  :disabled="busyId === u.id"
                  :aria-label="`Ubah peran ${u.username}`"
                  @click="toggleRole(u)"
                >
                  {{ u.role === 'admin' ? 'Admin' : 'Guru' }}
                </button>
              </td>
              <td data-label="Status">
                <StatusDot
                  :variant="u.disabled ? 'dot-muted' : 'dot-success'"
                  :label="u.disabled ? 'Nonaktif' : 'Aktif'"
                />
              </td>
              <td data-label="Dibuat" class="muted tnum">{{ formatDate(u.created_at) }}</td>
              <td class="actions">
                <template v-if="resettingId === u.id">
                  <input
                    v-model="newPassword"
                    class="input pw-input"
                    type="password"
                    placeholder="Kata sandi baru"
                    autocomplete="new-password"
                  />
                  <button class="btn btn-primary btn-sm" type="button" :disabled="busyId === u.id" @click="saveReset(u)">
                    Simpan
                  </button>
                  <button class="btn btn-ghost btn-sm" type="button" @click="resettingId = null">
                    Batal
                  </button>
                </template>
                <template v-else>
                  <button class="btn btn-ghost btn-sm" type="button" :disabled="busyId === u.id" @click="startReset(u)">
                    Ganti sandi
                  </button>
                  <button class="btn btn-ghost btn-sm" type="button" :disabled="busyId === u.id" @click="toggleDisabled(u)">
                    <AppIcon :name="u.disabled ? 'user-check' : 'x'" :size="15" />
                    <span>{{ u.disabled ? 'Aktifkan' : 'Nonaktifkan' }}</span>
                  </button>
                  <button class="btn btn-ghost btn-sm danger-text" type="button" :disabled="busyId === u.id" @click="remove(u)">
                    Hapus
                  </button>
                </template>
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

.create-form {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr auto;
  gap: var(--space-md);
  align-items: end;
}

.form-actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.cell-name {
  font-weight: var(--font-medium);
}

/* Role reads as a control (it toggles), so it wears an interactive affordance
 * rather than looking like static text. */
.role-btn {
  color: var(--text-link);
  font-weight: var(--font-medium);
}
.role-btn:hover {
  background: var(--info-bg);
}

.rows {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
  padding: var(--space-md) var(--panel-pad-x);
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

.pw-input {
  width: 10rem;
}

.danger-text {
  color: var(--text-error);
}
.danger-text:hover {
  background: var(--error-bg);
}

@media (max-width: 900px) {
  .create-form {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 640px) {
  .page-head {
    flex-direction: column;
    align-items: stretch;
  }
  .page-head > .btn {
    width: 100%;
  }
  .actions {
    flex-direction: column;
    align-items: stretch;
  }
}
</style>
