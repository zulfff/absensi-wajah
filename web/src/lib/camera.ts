// Camera capture helper. Wraps getUserMedia, a hidden <video>, and a canvas
// that emits base64 JPEG frames ready to POST to the enroll/kiosk endpoints.

export interface FrameSize {
  width: number
  height: number
}

export class Camera {
  private stream: MediaStream | null = null
  private video: HTMLVideoElement | null = null

  /** Attach to a <video> element and start the camera. */
  async start(video: HTMLVideoElement, size: FrameSize = { width: 640, height: 480 }): Promise<void> {
    this.video = video
    this.stream = await navigator.mediaDevices.getUserMedia({
      video: {
        width: { ideal: size.width },
        height: { ideal: size.height },
        facingMode: 'user',
      },
      audio: false,
    })
    video.srcObject = this.stream
    await video.play()
  }

  stop(): void {
    this.stream?.getTracks().forEach((t) => t.stop())
    this.stream = null
    if (this.video) this.video.srcObject = null
  }

  get active(): boolean {
    return this.stream !== null
  }

  /**
   * Grab the current frame as a base64 JPEG (no data-URL prefix).
   *
   * `quality` 0..1 — 0.8 keeps a 640x480 face well under the server's 6 MB
   * body cap while preserving enough detail for detection.
   */
  capture(quality = 0.8): string | null {
    const video = this.video
    if (!video || video.videoWidth === 0) return null

    const canvas = document.createElement('canvas')
    canvas.width = video.videoWidth
    canvas.height = video.videoHeight
    const ctx = canvas.getContext('2d')
    if (!ctx) return null
    // Mirror to match the on-screen preview, so what the user sees framed is
    // what the model sees.
    ctx.translate(canvas.width, 0)
    ctx.scale(-1, 1)
    ctx.drawImage(video, 0, 0, canvas.width, canvas.height)

    const url = canvas.toDataURL('image/jpeg', quality)
    return url.split(',')[1] ?? null
  }
}

/** Human-friendly message for a getUserMedia failure. */
export function cameraErrorMessage(err: unknown): string {
  if (err instanceof DOMException) {
    switch (err.name) {
      case 'NotAllowedError':
        return 'Akses kamera ditolak. Izinkan kamera di pengaturan browser.'
      case 'NotFoundError':
        return 'Tidak ada kamera yang terdeteksi.'
      case 'NotReadableError':
        return 'Kamera sedang dipakai aplikasi lain.'
      case 'OverconstrainedError':
        return 'Kamera tidak mendukung resolusi yang diminta.'
      default:
        return `Kamera gagal dibuka: ${err.message}`
    }
  }
  return 'Kamera gagal dibuka.'
}
