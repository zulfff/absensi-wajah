<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { cachedUserRole, currentRole, isAuthed, setToken } from './lib/api'
import AppIcon from './components/AppIcon.vue'

const route = useRoute()
const router = useRouter()

// Bare routes (login, kiosk) render without the admin chrome.
const bare = computed(() => route.meta.bare === true)

// Role drives which nav items appear. Populated after login / on load; the
// admin-only entry is hidden (never shown-then-forbidden) for a guru.
const role = ref<string | null>(cachedUserRole())

const baseNav = [
  { to: '/dashboard', label: 'Ringkasan', icon: 'gauge' },
  { to: '/students', label: 'Siswa', icon: 'users' },
  { to: '/attendance', label: 'Absensi', icon: 'calendar-check' },
  { to: '/devices', label: 'Perangkat', icon: 'monitor' },
]

const nav = computed(() =>
  role.value === 'admin'
    ? [...baseNav, { to: '/users', label: 'Akun', icon: 'shield-check' }]
    : baseNav,
)

const theme = ref<'light' | 'dark'>(
  (localStorage.getItem('absensi.theme') as 'light' | 'dark' | null) ?? 'light',
)

function applyTheme(): void {
  document.documentElement.setAttribute('data-theme', theme.value)
  localStorage.setItem('absensi.theme', theme.value)
}

function toggleTheme(): void {
  theme.value = theme.value === 'light' ? 'dark' : 'light'
  applyTheme()
}

function logout(): void {
  setToken(null)
  role.value = null
  router.push('/login')
}

onMounted(async () => {
  applyTheme()
  if (isAuthed()) {
    role.value = await currentRole()
  }
})
</script>

<template>
  <div v-if="bare" class="bare">
    <router-view />
  </div>

  <div v-else class="shell">
    <!-- Desktop / tablet: sidebar. Mobile: hidden, replaced by bottom nav. -->
    <aside class="sidebar" aria-label="Navigasi utama">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true"><AppIcon name="scan-face" :size="20" /></span>
        <span class="brand-name">Absensi&nbsp;Wajah</span>
      </div>

      <nav class="nav">
        <router-link
          v-for="item in nav"
          :key="item.to"
          :to="item.to"
          class="nav-item"
          active-class="is-active"
        >
          <AppIcon :name="item.icon" :size="18" />
          <span>{{ item.label }}</span>
        </router-link>
      </nav>

      <div class="sidebar-foot">
        <button class="btn btn-ghost btn-sm" type="button" @click="toggleTheme">
          <AppIcon :name="theme === 'light' ? 'moon' : 'sun'" :size="16" />
          <span>{{ theme === 'light' ? 'Mode gelap' : 'Mode terang' }}</span>
        </button>
        <button v-if="isAuthed()" class="btn btn-ghost btn-sm" type="button" @click="logout">
          <AppIcon name="log-out" :size="16" />
          <span>Keluar</span>
        </button>
      </div>
    </aside>

    <main class="content">
      <router-view />
    </main>

    <!-- Mobile: bottom navigation in the thumb zone. Theme + logout live in a
         top bar on this breakpoint, so the destructive action is not under the
         thumb (per the responsive contract). -->
    <header class="mobile-bar">
      <span class="mobile-brand">
        <span class="brand-mark" aria-hidden="true"><AppIcon name="scan-face" :size="18" /></span>
        <span>Absensi&nbsp;Wajah</span>
      </span>
      <div class="mobile-actions">
        <button
          class="icon-btn"
          type="button"
          :aria-label="theme === 'light' ? 'Mode gelap' : 'Mode terang'"
          @click="toggleTheme"
        >
          <AppIcon :name="theme === 'light' ? 'moon' : 'sun'" :size="18" />
        </button>
        <button v-if="isAuthed()" class="icon-btn" type="button" aria-label="Keluar" @click="logout">
          <AppIcon name="log-out" :size="18" />
        </button>
      </div>
    </header>

    <nav class="bottom-nav" aria-label="Navigasi utama">
      <router-link
        v-for="item in nav"
        :key="item.to"
        :to="item.to"
        class="bottom-item"
        active-class="is-active"
      >
        <AppIcon :name="item.icon" :size="20" />
        <span>{{ item.label }}</span>
      </router-link>
    </nav>
  </div>
</template>

<style scoped>
.bare {
  height: 100%;
}

.shell {
  display: grid;
  grid-template-columns: 15rem 1fr;
  height: 100%;
}

/* Sidebar / bottom-nav / mobile-bar are toggled per breakpoint below. */
.sidebar {
  display: flex;
  flex-direction: column;
  gap: var(--space-lg);
  padding: var(--space-lg) var(--space-md);
  background: var(--surface-sidebar);
  border-right: 1px solid var(--border-subtle);
  overscroll-behavior: contain;
}

.brand {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: 0 var(--space-sm);
}

.brand-mark {
  display: grid;
  place-items: center;
  width: 2rem;
  height: 2rem;
  border-radius: var(--radius-md);
  background: var(--accent-bg);
  color: var(--accent-text);
  flex: none;
}

.brand-name {
  font-size: var(--text-sm);
  font-weight: var(--font-semibold);
  letter-spacing: var(--tracking-tight);
}

.nav {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  flex: 1;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: 0.5rem var(--space-sm);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-size: var(--text-sm);
  font-weight: var(--font-medium);
  transition:
    background-color var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.nav-item:hover {
  background: var(--surface-sunken);
  color: var(--text-primary);
  text-decoration: none;
}

.nav-item.is-active {
  background: color-mix(in oklab, var(--accent-bg) 12%, transparent);
  color: var(--text-link);
}

.sidebar-foot {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  border-top: 1px solid var(--border-subtle);
  padding-top: var(--space-md);
}

.content {
  overflow-y: auto;
  padding: var(--space-xl);
  /* Keep scroll chaining local to the content pane. */
  overscroll-behavior: contain;
}

/* Mobile chrome is hidden on desktop. */
.mobile-bar,
.bottom-nav {
  display: none;
}

.icon-btn {
  display: grid;
  place-items: center;
  width: var(--touch-target);
  height: var(--touch-target);
  border: 0;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.icon-btn:hover {
  background: var(--surface-sunken);
  color: var(--text-primary);
}

/* ---- Tablet: collapse to an icon rail (1024px and below) ---- */
@media (max-width: 1024px) {
  .shell {
    grid-template-columns: 3.5rem 1fr;
  }
  .brand-name,
  .nav-item span,
  .sidebar-foot span {
    display: none;
  }
  .nav-item,
  .brand,
  .sidebar-foot button {
    justify-content: center;
  }
  .content {
    padding: var(--space-lg);
  }
}

/* ---- Mobile: no sidebar, bottom nav + top bar (768px and below) ---- */
@media (max-width: 768px) {
  .shell {
    display: block;
    /* Content scrolls; the chrome is fixed to the viewport edges. */
    height: 100%;
  }
  .sidebar {
    display: none;
  }

  .content {
    height: 100%;
    padding: var(--space-md);
    /* Clear the fixed top bar and bottom nav, plus the home indicator. */
    padding-top: calc(3.25rem + var(--safe-top));
    padding-bottom: calc(4.5rem + var(--safe-bottom));
    padding-left: calc(var(--space-md) + var(--safe-left));
    padding-right: calc(var(--space-md) + var(--safe-right));
    -webkit-overflow-scrolling: touch;
  }

  .mobile-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    position: fixed;
    inset: 0 0 auto 0;
    height: calc(3.25rem + var(--safe-top));
    padding: var(--safe-top) var(--space-md) 0;
    padding-left: calc(var(--space-md) + var(--safe-left));
    padding-right: calc(var(--space-md) + var(--safe-right));
    background: var(--surface-sidebar);
    border-bottom: 1px solid var(--border-subtle);
    z-index: var(--z-sticky);
  }

  .mobile-brand {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-sm);
    font-weight: var(--font-semibold);
    letter-spacing: var(--tracking-tight);
  }

  .mobile-actions {
    display: flex;
    gap: var(--space-xs);
  }

  .bottom-nav {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    position: fixed;
    inset: auto 0 0 0;
    /* Thumb zone; respect the home indicator and side notches in landscape. */
    padding: 0.35rem calc(var(--space-xs) + var(--safe-right))
      calc(0.35rem + var(--safe-bottom))
      calc(var(--space-xs) + var(--safe-left));
    background: var(--surface-sidebar);
    border-top: 1px solid var(--border-subtle);
    z-index: var(--z-sticky);
  }

  .bottom-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    min-height: var(--touch-target);
    padding: var(--space-xs);
    border-radius: var(--radius-md);
    color: var(--text-tertiary);
    font-size: 0.6875rem;
    font-weight: var(--font-medium);
    text-align: center;
  }
  .bottom-item:hover {
    text-decoration: none;
  }
  .bottom-item.is-active {
    color: var(--text-link);
  }
}
</style>
