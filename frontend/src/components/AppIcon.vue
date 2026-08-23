<script setup lang="ts">
const { name } = defineProps<{
  name:
    | "open"
    | "close"
    | "left"
    | "right"
    | "minus"
    | "plus"
    | "shield"
    | "document"
    | "grid"
    | "wifi-off"
    | "clock"
    | "lock";
}>();

const paths = {
  open: ["M3 6.5h6l2-2h10v14H3z", "M3 9.5h18"],
  close: ["m7 7 10 10", "M17 7 7 17"],
  left: ["m15 6-6 6 6 6"],
  right: ["m9 6 6 6-6 6"],
  minus: ["M6 12h12"],
  plus: ["M12 6v12", "M6 12h12"],
  shield: ["M12 3 5 6v5c0 4.6 2.9 8.2 7 10 4.1-1.8 7-5.4 7-10V6z", "M9 12l2 2 4-4"],
  document: ["M7 3h7l4 4v14H7z", "M14 3v4h4", "M9 13h6", "M9 17h6"],
  grid: ["M4.5 4.5h6v6h-6z", "M13.5 4.5h6v6h-6z", "M4.5 13.5h6v6h-6z", "M13.5 13.5h6v6h-6z"],
  "wifi-off": ["M2 8a14 14 0 0 1 5.2-3", "M7.5 12a8.5 8.5 0 0 1 3.2-1.7", "M10.8 15.7 12 16.5a4.5 4.5 0 0 1 2 .5", "M12 20h.01", "M4 3l16 18"],
  clock: ["M12 4.6a7.4 7.4 0 1 0 0 14.8 7.4 7.4 0 0 0 0-14.8z", "M12 8.2v4l2.6 1.6"],
  lock: ["M6.5 11h11v8.5h-11z", "M9 11V8.6a3 3 0 0 1 6 0V11", "M12 14.6v2"],
};
</script>

<template>
  <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
    <path v-for="path in paths[name]" :key="path" :d="path" />
  </svg>
</template>
