import { createApp } from 'vue'
import { createRouter, createWebHashHistory } from 'vue-router'
import App from './App.vue'
import { isAuthed, currentRole } from './lib/api'

import './styles/tokens.css'
import './styles/base.css'
import './styles/components.css'

import LoginView from './views/LoginView.vue'
import DashboardView from './views/DashboardView.vue'
import StudentsView from './views/StudentsView.vue'
import EnrollView from './views/EnrollView.vue'
import DevicesView from './views/DevicesView.vue'
import AttendanceView from './views/AttendanceView.vue'
import UsersView from './views/UsersView.vue'
import KioskView from './views/KioskView.vue'

// The kiosk hostname opens straight into the kiosk screen so a student device
// never sees the login. Any other host lands on the admin dashboard.
function isKioskHost(): boolean {
  return location.hostname.startsWith('absensi.')
}

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: () => (isKioskHost() ? '/kiosk' : '/dashboard') },
    { path: '/login', component: LoginView, meta: { public: true, bare: true } },
    { path: '/kiosk', component: KioskView, meta: { public: true, bare: true } },
    { path: '/dashboard', component: DashboardView },
    { path: '/students', component: StudentsView },
    { path: '/students/:id/enroll', component: EnrollView },
    { path: '/devices', component: DevicesView },
    { path: '/attendance', component: AttendanceView },
    { path: '/users', component: UsersView, meta: { adminOnly: true } },
    { path: '/:pathMatch(.*)*', redirect: () => (isKioskHost() ? '/kiosk' : '/dashboard') },
  ],
})

// Auth gate: any non-public route requires a token. Admin-only routes also
// require the admin role (checked against the live user, not a claim).
router.beforeEach(async (to) => {
  if (to.meta.public) return true
  if (!isAuthed()) return { path: '/login' }
  if (to.meta.adminOnly) {
    const role = await currentRole()
    if (role !== 'admin') return { path: '/dashboard' }
  }
  return true
})

createApp(App).use(router).mount('#app')
