<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { login } from '../lib/api'
import { ApiError } from '../lib/types'
import AppIcon from '../components/AppIcon.vue'

const router = useRouter()
const username = ref('')
const password = ref('')
const error = ref('')
const loading = ref(false)

async function submit(): Promise<void> {
  error.value = ''
  loading.value = true
  try {
    await login(username.value.trim(), password.value)
    router.push('/dashboard')
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'Gagal masuk.'
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="login-wrap">
    <div class="card card-pad login-card">
      <div class="login-head">
        <span class="brand-mark" aria-hidden="true"><AppIcon name="scan-face" :size="22" /></span>
        <h1>Masuk admin</h1>
        <p class="sub">Kelola siswa, pendaftaran wajah, dan perangkat kiosk.</p>
      </div>

      <form class="login-form" @submit.prevent="submit">
        <div class="field">
          <label class="label" for="username">Nama pengguna</label>
          <input
            id="username"
            v-model="username"
            class="input"
            type="text"
            autocomplete="username"
            :class="{ 'has-error': error }"
            required
          />
        </div>

        <div class="field">
          <label class="label" for="password">Kata sandi</label>
          <input
            id="password"
            v-model="password"
            class="input"
            type="password"
            autocomplete="current-password"
            :class="{ 'has-error': error }"
            required
          />
        </div>

        <p v-if="error" class="alert alert-error" role="alert">
          <AppIcon name="alert" :size="16" />
          <span>{{ error }}</span>
        </p>

        <button class="btn btn-primary login-btn" type="submit" :disabled="loading">
          <span v-if="loading">Memproses…</span>
          <span v-else>Masuk</span>
        </button>
      </form>

      <p class="kiosk-hint">
        Halaman kiosk: <a class="kiosk-link" href="#/kiosk">buka kiosk</a>
      </p>
    </div>
  </div>
</template>

<style scoped>
.login-wrap {
  min-height: 100%;
  display: grid;
  place-items: center;
  padding: var(--space-xl);
  /* Flat canvas, not a full-bleed visual: the card is the whole composition
   * (stacked), so it must not sit on a viewport-spanning background image. */
  background: var(--surface-canvas);
}

.login-card {
  width: min(24rem, 100%);
  box-shadow: var(--shadow-lg);
}

.login-head {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-sm);
  margin-bottom: var(--space-lg);
}

.brand-mark {
  display: grid;
  place-items: center;
  width: 2.5rem;
  height: 2.5rem;
  border-radius: var(--radius-md);
  background: var(--accent-bg);
  color: var(--accent-text);
}

.login-head h1 {
  font-size: var(--text-2xl);
}

.sub {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
}

.login-btn {
  margin-top: var(--space-xs);
  justify-content: center;
}

.kiosk-hint {
  margin-top: var(--space-lg);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  text-align: center;
}

/* The "buka kiosk" link is inline, so its tap height is just the line box
 * (~14px). Give it a real touch target without changing the visual rhythm. */
.kiosk-link {
  display: inline-block;
  padding: var(--space-sm) var(--space-xs);
  margin: calc(-1 * var(--space-sm)) calc(-1 * var(--space-xs));
}
</style>
