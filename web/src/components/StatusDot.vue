<script setup lang="ts">
// A 6px status dot + label. Status uses a dot, never a badge/pill.
defineProps<{
  variant: string
  label: string
}>()
</script>

<template>
  <span class="status-line">
    <span class="dot" :class="variant" />
    <span>{{ label }}</span>
  </span>
</template>
