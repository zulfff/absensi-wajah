// Small formatting helpers shared across views.

/** Format an ISO timestamp as local `HH:MM:SS`. */
export function formatTime(iso: string): string {
  const d = new Date(iso)
  return d.toLocaleTimeString('id-ID', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

/** Format an ISO date as `D Mon YYYY`. */
export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString('id-ID', { day: 'numeric', month: 'short', year: 'numeric' })
}

/** A 0..1 score as a percentage string with one decimal. */
export function percent(value: number): string {
  return `${(value * 100).toFixed(1)}%`
}

/** Today's date as `YYYY-MM-DD` in local time (for the attendance query). */
export function todayISO(): string {
  const d = new Date()
  const off = d.getTimezoneOffset()
  return new Date(d.getTime() - off * 60_000).toISOString().slice(0, 10)
}

/** Map a backend attempt outcome to a plain-words label. */
export function outcomeLabel(outcome: string): string {
  const map: Record<string, string> = {
    accepted: 'Diterima',
    rejected_unknown: 'Ditolak — tidak dikenali',
    rejected_margin: 'Ditolak — margin kecil',
    rejected_liveness: 'Ditolak — liveness',
    retry_timeout: 'Waktu habis',
    cooldown_shown: 'Sudah absen',
    error: 'Error',
  }
  return map[outcome] ?? outcome
}

/** Stable short id for display (first 8 chars). */
export function shortId(id: string): string {
  return id.slice(0, 8)
}
