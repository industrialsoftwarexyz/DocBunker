<script setup lang="ts">
import { ref, watch } from "vue";
import type { DocumentHandleDto, DocumentInfoDto, RenderedPageDto } from "../types";
import { renderPage } from "../api";

const props = defineProps<{
  handle: DocumentHandleDto;
  info: DocumentInfoDto;
  currentPage: number;
}>();

const emit = defineEmits<{
  "select:page": [page: number];
}>();

const THUMB_WIDTH = 150 * (window.devicePixelRatio || 1);
const pages = ref<(RenderedPageDto | null)[]>([]);
const rendered = ref<Set<number>>(new Set());
const renderQueue = ref<number[]>([]);
const processing = ref(false);

function heightForWidth(width: number, info: DocumentInfoDto): number {
  return Math.round((width * info.height) / info.width);
}

function queueRender(pageIndex: number) {
  if (rendered.value.has(pageIndex)) return;
  if (renderQueue.value.includes(pageIndex)) return;
  renderQueue.value.push(pageIndex);
  processQueue();
}

async function processQueue() {
  if (processing.value || renderQueue.value.length === 0) return;
  processing.value = true;
  while (renderQueue.value.length > 0) {
    const pageIndex = renderQueue.value.shift()!;
    if (rendered.value.has(pageIndex)) continue;
    const targetHeight = heightForWidth(THUMB_WIDTH, props.info);
    try {
      const result = await renderPage(props.handle, pageIndex, THUMB_WIDTH, targetHeight);
      pages.value[pageIndex] = result;
      rendered.value = new Set(rendered.value).add(pageIndex);
    } catch {
      // silently ignore render failures for thumbnails
    }
  }
  processing.value = false;
}

watch(
  () => props.info,
  (info) => {
    if (!info) return;
    pages.value = new Array(info.page_count).fill(null);
    rendered.value = new Set();
    renderQueue.value = [];
    for (let i = 0; i < Math.min(info.page_count, 20); i++) {
      queueRender(i);
    }
  },
  { immediate: true },
);

watch(
  () => props.currentPage,
  (page) => {
    const idx = page - 1;
    if (idx >= 0 && props.info) {
      const start = Math.max(0, idx - 5);
      const end = Math.min(props.info.page_count, idx + 15);
      for (let i = start; i < end; i++) {
        queueRender(i);
      }
    }
  },
  { immediate: true },
);
</script>

<template>
  <aside class="thumbnail-sidebar" role="navigation" aria-label="Miniaturas de páginas">
    <div class="thumbnail-list">
      <button
        v-for="(_, index) in info.page_count"
        :key="index"
        class="thumbnail-item"
        :class="{ active: currentPage === index + 1 }"
        :aria-label="`Página ${index + 1}`"
        :aria-current="currentPage === index + 1 ? 'page' : undefined"
        @click="emit('select:page', index + 1)"
      >
        <div class="thumbnail-frame">
          <img
            v-if="pages[index]"
            :src="pages[index]!.data_url"
            :alt="`Miniatura página ${index + 1}`"
            draggable="false"
          />
          <div v-else class="thumbnail-placeholder" />
        </div>
        <span class="thumbnail-number">{{ index + 1 }}</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.thumbnail-sidebar {
  inline-size: 180px;
  block-size: 100%;
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-width: thin;
  scrollbar-color: var(--border-strongest) transparent;
  background: var(--surface-muted);
  border-inline-end: 1px solid var(--border-strong);
}

.thumbnail-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
}

.thumbnail-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 4px;
  cursor: pointer;
  border: 2px solid transparent;
  border-radius: var(--radius);
  background: transparent;
  transition: border-color 90ms ease, background 90ms ease;
}

.thumbnail-item:hover {
  background: var(--accent-soft);
}

.thumbnail-item.active {
  border-color: var(--accent-border);
  background: var(--accent-soft);
}

.thumbnail-frame {
  inline-size: 140px;
  block-size: auto;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--surface-raised);
}

.thumbnail-frame img {
  display: block;
  inline-size: 100%;
  block-size: auto;
  user-select: none;
  pointer-events: none;
}

.thumbnail-placeholder {
  inline-size: 100%;
  aspect-ratio: 0.707;
  background: var(--surface-raised);
  animation: thumbnail-pulse 1.4s ease-in-out infinite;
}

@keyframes thumbnail-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.55;
  }
}

.thumbnail-number {
  font-size: 11px;
  font-weight: 500;
  color: var(--text-muted);
}

.thumbnail-item.active .thumbnail-number {
  color: var(--link);
  font-weight: 600;
}
</style>
