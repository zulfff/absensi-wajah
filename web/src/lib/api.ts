// HTTP client for the admin API. Holds the JWT in memory + localStorage and
// attaches it to every request.
//
// The token is deliberately NOT put in a cookie: cookies would be sent
// automatically on cross-site requests (CSRF surface). A bearer token the app
// attaches explicitly is the safer choice for this API.

import { ref } from 'vue'
import { ApiError, type AttendanceRow, type CommitResponse, type Device, type DeviceCreated, type FrameFeedback, type LoginResponse, type MonitoringSummary, type Student, type StudentDetail, type StudentListItem, type User } from './types'

const TOKEN_KEY = 'absensi.token'

let token: string | null = localStorage.getItem(TOKEN_KEY)

/** Reactive mirror of `token`, so templates re-render on login/logout/401. */
export const authed = ref(token !== null)

/// The signed-in user's role, cached for the session. `null` until fetched.
let cachedRole: string | null = null

export function setToken(value: string | null): void {
  token = value
  cachedRole = null
  authed.value = value !== null
  if (value) localStorage.setItem(TOKEN_KEY, value)
  else localStorage.removeItem(TOKEN_KEY)
}

export function getToken(): string | null {
  return token
}

export function isAuthed(): boolean {
  return token !== null
}

/** Fetch (and cache) the current user's live role. */
export async function currentRole(): Promise<string | null> {
  if (!token) return null
  if (cachedRole) return cachedRole
  try {
    const me = await request<{ username: string; role: string }>('/api/auth/me')
    cachedRole = me.role
    return cachedRole
  } catch {
    return null
  }
}

/** Role cached so far, without a network call. */
export function cachedUserRole(): string | null {
  return cachedRole
}

/** Drop cached identity (on logout or 401). */
export function clearIdentity(): void {
  cachedRole = null
}

/** Clear the session and bounce to login. */
function onUnauthorized(): void {
  setToken(null)
  if (location.hash !== '#/login') location.hash = '#/login'
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers)
  if (token) headers.set('authorization', `Bearer ${token}`)
  if (init.body && !headers.has('content-type')) {
    headers.set('content-type', 'application/json')
  }

  const res = await fetch(path, { ...init, headers })

  if (res.status === 401) {
    onUnauthorized()
    throw new ApiError('unauthorized', 'Sesi berakhir. Silakan masuk lagi.', 401)
  }

  if (!res.ok) {
    let code = 'error'
    let message = `Permintaan gagal (${res.status})`
    try {
      const body = await res.json()
      if (body && typeof body === 'object') {
        code = body.code ?? code
        message = body.message ?? message
      }
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(code, message, res.status)
  }

  if (res.status === 204) return undefined as T
  const ct = res.headers.get('content-type') ?? ''
  if (!ct.includes('application/json')) return undefined as T
  return (await res.json()) as T
}

// ---- auth ----

export async function login(username: string, password: string): Promise<LoginResponse> {
  const res = await request<LoginResponse>('/api/auth/login', {
    method: 'POST',
    body: JSON.stringify({ username, password }),
  })
  setToken(res.token)
  return res
}

// ---- users ----

export function listUsers(): Promise<User[]> {
  return request<User[]>('/api/users')
}

export function createUser(input: { username: string; password: string; role: string }): Promise<User> {
  return request<User>('/api/users', { method: 'POST', body: JSON.stringify(input) })
}

export function updateUser(
  id: string,
  input: { role?: string; disabled?: boolean; password?: string },
): Promise<User> {
  return request<User>(`/api/users/${id}`, { method: 'PUT', body: JSON.stringify(input) })
}

export function deleteUser(id: string): Promise<void> {
  return request<void>(`/api/users/${id}`, { method: 'DELETE' })
}

// ---- students ----

export function listStudents(limit = 200, offset = 0): Promise<StudentListItem[]> {
  return request<StudentListItem[]>(`/api/students?limit=${limit}&offset=${offset}`)
}

export function getStudent(id: string): Promise<StudentDetail> {
  return request<StudentDetail>(`/api/students/${id}`)
}

export function createStudent(input: { nis: string; nama: string; kelas_id?: string | null }): Promise<Student> {
  return request<Student>('/api/students', { method: 'POST', body: JSON.stringify(input) })
}

export function updateStudent(id: string, input: { nama?: string; status?: string; kelas_id?: string | null }): Promise<Student> {
  return request<Student>(`/api/students/${id}`, { method: 'PUT', body: JSON.stringify(input) })
}

export function deleteStudent(id: string): Promise<void> {
  return request<void>(`/api/students/${id}`, { method: 'DELETE' })
}

/** Record parental/guardian consent. Required before any enrollment call. */
export function setConsent(id: string, consentBy?: string): Promise<Student> {
  return request<Student>(`/api/students/${id}/consent`, {
    method: 'POST',
    body: JSON.stringify({ consent_by: consentBy ?? '' }),
  })
}

// ---- enrollment ----

/** Send one frame for quality feedback. `image` is a base64 JPEG. */
export function enrollFrame(id: string, image: string, step: number): Promise<FrameFeedback> {
  return request<FrameFeedback>(`/api/students/${id}/enroll/frame`, {
    method: 'POST',
    body: JSON.stringify({ image, step }),
  })
}

export function enrollCommit(id: string, frames: string[]): Promise<CommitResponse> {
  return request<CommitResponse>(`/api/students/${id}/enroll/commit`, {
    method: 'POST',
    body: JSON.stringify({ frames }),
  })
}

export function activateStudent(id: string): Promise<{ activated_templates: number; gallery_students: number }> {
  return request(`/api/students/${id}/activate`, { method: 'POST' })
}

export function deactivateStudent(id: string): Promise<{ ok: boolean }> {
  return request(`/api/students/${id}/deactivate`, { method: 'POST' })
}

export function deleteFace(id: string): Promise<{ deleted_templates: number }> {
  return request(`/api/students/${id}/face`, { method: 'DELETE' })
}

// ---- devices ----

export function listDevices(): Promise<Device[]> {
  return request<Device[]>('/api/devices')
}

export function createDevice(input: { nama: string; lokasi?: string }): Promise<DeviceCreated> {
  return request<DeviceCreated>('/api/devices', { method: 'POST', body: JSON.stringify(input) })
}

export function revokeDevice(id: string): Promise<void> {
  return request<void>(`/api/devices/${id}/revoke`, { method: 'POST' })
}

export function deleteDevice(id: string): Promise<void> {
  return request<void>(`/api/devices/${id}`, { method: 'DELETE' })
}

// ---- attendance & monitoring ----

export function listAttendance(params: { tanggal?: string; kelas_id?: string } = {}): Promise<AttendanceRow[]> {
  const q = new URLSearchParams()
  if (params.tanggal) q.set('tanggal', params.tanggal)
  if (params.kelas_id) q.set('kelas_id', params.kelas_id)
  const qs = q.toString()
  return request<AttendanceRow[]>(`/api/attendance${qs ? `?${qs}` : ''}`)
}

export function correctAttendance(id: string, note?: string): Promise<void> {
  return request<void>(`/api/attendance/${id}/correct`, {
    method: 'POST',
    body: JSON.stringify({ note: note ?? '' }),
  })
}

/** Teacher fallback: record presence by hand when the camera cannot. */
export function markAttendanceManual(studentId: string, note?: string): Promise<unknown> {
  return request(`/api/attendance`, {
    method: 'POST',
    body: JSON.stringify({ student_id: studentId, note: note ?? '' }),
  })
}

export function monitoringSummary(): Promise<MonitoringSummary> {
  return request<MonitoringSummary>('/api/monitoring/summary')
}

export function reloadGallery(): Promise<{ students: number }> {
  return request('/api/gallery/reload', { method: 'POST' })
}
