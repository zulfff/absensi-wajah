// API types mirroring the Rust DTOs. Kept hand-written and small; the backend
// is the source of truth and these mirror its serde shapes.

export interface Student {
  id: string
  nis: string
  nama: string
  kelas_id: string | null
  status: string
  consent_granted: boolean
  created_at: string
}

/** Student as returned by the list endpoint: the row plus enrollment counts. */
export interface StudentListItem extends Student {
  template_count: number
  active_template_count: number
}

export interface StudentDetail {
  student: Student
  template_count: number
}

export interface Device {
  id: string
  nama: string
  lokasi: string | null
  revoked: boolean
  last_seen: string | null
  created_at: string
}

export interface DeviceCreated {
  device: Device
  token: string
}

export interface LoginResponse {
  token: string
  role: string
  username: string
}

export interface User {
  id: string
  username: string
  role: 'admin' | 'guru'
  disabled: boolean
  created_at: string
}

export interface FrameFeedback {
  accepted: boolean
  score: number
  reason: string | null
  guidance: string
}

export type CommitResponse =
  | { status: 'accepted'; templates_created: number; dropped_frames: number; nearest_neighbour_similarity: number; pending_activation: boolean }
  | { status: 'duplicate'; conflicts_with: string; similarity: number }
  | { status: 'risky_separation'; nearest_neighbour_id: string; similarity: number }
  | { status: 'insufficient_frames'; kept: number; needed: number }
  | { status: 'no_frames' }

export interface AttendanceRow {
  id: string
  student_id: string
  nama: string
  nis: string
  timestamp: string
  similarity: number
  margin: number
  liveness_score: number
  status: string
}

export interface MonitoringSummary {
  outcomes: Array<{
    outcome: string
    count: number
    avg_top1: number | null
    avg_margin: number | null
  }>
  enrolled_students: number
  gallery_students: number
}

/** A typed API error carrying the backend's stable `code`. */
export class ApiError extends Error {
  code: string
  status: number
  constructor(code: string, message: string, status: number) {
    super(message)
    this.code = code
    this.status = status
  }
}
