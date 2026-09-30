<script setup lang="ts">
import { computed } from 'vue'

// Inline SVG sparkline — the recipe says sparklines are a polyline, no chart
// library. Single accent hue, gradient fill fading to zero (never a rainbow).
const props = withDefaults(
  defineProps<{
    values: number[]
    width?: number
    height?: number
    label?: string
  }>(),
  { width: 320, height: 44, label: 'Tren' },
)

const geometry = computed(() => {
  const vs = props.values.length > 0 ? props.values : [0]
  const max = Math.max(...vs, 1)
  const n = vs.length
  const stepX = n > 1 ? props.width / (n - 1) : 0
  const points = vs.map((v, i) => {
    const x = i * stepX
    // Leave 2px headroom so the peak is not clipped by the stroke.
    const y = props.height - 2 - (v / max) * (props.height - 4)
    return [x, y] as const
  })
  const line = points.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' ')
  const area = `M0,${props.height} L${points.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' L')} L${props.width},${props.height} Z`
  return { line, area }
})
</script>

<template>
  <svg
    class="sparkline"
    :width="width"
    :height="height"
    :viewBox="`0 0 ${width} ${height}`"
    :style="{ maxWidth: `${width}px` }"
    role="img"
    :aria-label="label"
    preserveAspectRatio="none"
  >
    <defs>
      <linearGradient :id="`spark-${label.length}-${width}`" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="var(--accent-bg)" stop-opacity="0.18" />
        <stop offset="100%" stop-color="var(--accent-bg)" stop-opacity="0" />
      </linearGradient>
    </defs>
    <path :d="geometry.area" :fill="`url(#spark-${label.length}-${width})`" />
    <polyline
      :points="geometry.line"
      fill="none"
      stroke="var(--accent-bg)"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
</template>

<style scoped>
.sparkline {
  display: block;
  /* Fluid: never wider than its column. The `width` attribute is the intrinsic
   * size; without this the fixed 320px SVG overflowed a 288px phone column and
   * made the whole content pane scroll sideways. */
  width: 100%;
  overflow: visible;
}
</style>
