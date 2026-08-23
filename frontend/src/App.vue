<script setup lang="ts">
import {
  computed,
  onMounted,
  onUnmounted,
  onWatcherCleanup,
  ref,
  useTemplateRef,
  watch,
} from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  closeDocument,
  getBackendStatus,
  getDocumentInfo,
  isCancelled,
  openDocument,
  openDocumentByPath,
  openStartupDocument,
  renderPage,
  safeMessage,
} from "./api";
import AppIcon from "./components/AppIcon.vue";
import PageThumbnails from "./components/PageThumbnails.vue";
import type {
  DocumentHandleDto,
  DocumentInfoDto,
  RenderedPageDto,
} from "./types";

const MAX_PAGE_DIM = 4096;
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
const DPR = window.devicePixelRatio || 1;
// Supersample factor: render pages at at least 1.5 device pixels per CSS
// pixel so text and hairlines stay crisp on 1x displays and when the page is
// downscaled. On high-DPI displays the device ratio already provides enough
// resolution, so no extra factor is added there (avoids 2-3x cost for no
// visible gain).
const RENDER_QUALITY = Math.min(1.5, Math.max(1, 1.5 / DPR));
const VIEWPORT_GUTTER = 64;

type FitMode = "fitWidth" | "fitPage";

const handle = ref<DocumentHandleDto | null>(null);
const info = ref<DocumentInfoDto | null>(null);
const fileLabel = ref<string | null>(null);
const page = ref(1);
const fitMode = ref<FitMode>("fitWidth");
const zoom = ref(1);
const renderedPage = ref<RenderedPageDto | null>(null);
const renderedPageNumber = ref(1);
const opening = ref(false);
const rendering = ref(false);
const panning = ref(false);
const error = ref<string | null>(null);
const backendAvailable = ref<boolean | null>(null);
const backendIsolated = ref<boolean | null>(null);
const viewportWidth = ref(0);
const viewportHeight = ref(0);
const renderSequence = ref(0);
const viewportElement = useTemplateRef<HTMLDivElement>("viewportElement");
const draggingOver = ref(false);
const showThumbnails = ref(false);
// Best-effort cache of the page rendered just ahead of the current one, so
// flipping forward is instant. Entries are keyed by page + target size and
// cleared on document change (bounded to a few entries).
const preloadedPages = ref<Map<string, RenderedPageDto>>(new Map());
let stopDragDropListener: UnlistenFn | null = null;

const hasDocument = computed(() => handle.value !== null && info.value !== null);
const pageCount = computed(() => info.value?.page_count ?? 0);
const pageWidth = computed(() => {
  const dimensions = targetDimensions.value;
  return dimensions ? dimensions.width / (DPR * RENDER_QUALITY) : undefined;
});
const pageHeight = computed(() => {
  const dimensions = targetDimensions.value;
  return dimensions ? dimensions.height / (DPR * RENDER_QUALITY) : undefined;
});
const canPan = computed(
  () =>
    hasDocument.value &&
    ((pageWidth.value ?? 0) + VIEWPORT_GUTTER > viewportWidth.value ||
      (pageHeight.value ?? 0) + VIEWPORT_GUTTER > viewportHeight.value),
);
const documentMeta = computed(() => {
  if (!info.value) return "PDF, imágenes y Office";
  const noun = info.value.page_count === 1 ? "página" : "páginas";
  return `${info.value.format.toUpperCase()} · ${info.value.page_count} ${noun}`;
});
const isolationLabel = computed(() => {
  if (backendAvailable.value === null) return "Comprobando aislamiento";
  if (!backendAvailable.value) return "Servicio no disponible";
  if (!backendIsolated.value) return "Aislamiento no disponible";
  return hasDocument.value ? "Documento aislado" : "Entorno aislado preparado";
});
const emptyStateCopy = computed(() => {
  if (backendAvailable.value === null) return "Seleccione un archivo PDF, una imagen o un documento de Office.";
  if (!backendAvailable.value) return "No se pudo iniciar el servicio de documentos.";
  if (!backendIsolated.value) return "Modo de desarrollo: los documentos no están aislados.";
  return "Seleccione un archivo PDF, una imagen o un documento de Office para verlo de forma segura.";
});
const targetDimensions = computed(() => {
  if (!info.value) return null;

  const availableWidth = Math.max(1, viewportWidth.value - VIEWPORT_GUTTER);
  const availableHeight = Math.max(1, viewportHeight.value - VIEWPORT_GUTTER);
  const fitScale =
    fitMode.value === "fitWidth"
      ? availableWidth / info.value.width
      : Math.min(
          availableWidth / info.value.width,
          availableHeight / info.value.height,
        );
  const requestedWidth = info.value.width * fitScale * zoom.value * DPR * RENDER_QUALITY;
  const requestedHeight = info.value.height * fitScale * zoom.value * DPR * RENDER_QUALITY;
  const limitScale = Math.min(
    1,
    MAX_PAGE_DIM / requestedWidth,
    MAX_PAGE_DIM / requestedHeight,
  );

  return {
    width: Math.max(1, Math.round(requestedWidth * limitScale)),
    height: Math.max(1, Math.round(requestedHeight * limitScale)),
  };
});

let resizeObserver: ResizeObserver | null = null;
let stopAssociatedFileListener: UnlistenFn | null = null;
let drainingStartupDocuments = false;
let startupDrainRequested = false;
let unmounted = false;
let failedRenderKey: string | null = null;
let panStart: {
  pointerId: number;
  x: number;
  y: number;
  scrollLeft: number;
  scrollTop: number;
} | null = null;

onMounted(() => {
  void getBackendStatus()
    .then((status) => {
      backendAvailable.value = status.available;
      backendIsolated.value = status.isolated;
    })
    .catch(() => {
      backendAvailable.value = false;
      backendIsolated.value = false;
    });
  const element = viewportElement.value;
  if (element) {
    resizeObserver = new ResizeObserver(([entry]) => {
      if (!entry) return;
      viewportWidth.value = entry.contentRect.width;
      viewportHeight.value = entry.contentRect.height;
    });
    resizeObserver.observe(element);
  }
  window.addEventListener("keydown", onKeyDown);
  void initializeAssociatedFileListener();
  void initializeDragDrop();
});

onUnmounted(() => {
  unmounted = true;
  resizeObserver?.disconnect();
  stopAssociatedFileListener?.();
  stopDragDropListener?.();
  window.removeEventListener("keydown", onKeyDown);
  if (handle.value) void closeDocument(handle.value).catch(() => undefined);
});

async function initializeAssociatedFileListener() {
  try {
    const unlisten = await listen("associated-file-ready", () => {
      void drainStartupDocuments();
    });
    if (unmounted) {
      unlisten();
      return;
    }
    stopAssociatedFileListener = unlisten;
    await drainStartupDocuments();
  } catch (cause) {
    if (!unmounted) error.value = safeMessage(cause);
  }
}

const SUPPORTED_EXTENSIONS = ["pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx"];

function hasSupportedExtension(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return SUPPORTED_EXTENSIONS.includes(ext);
}

async function initializeDragDrop() {
  try {
    const window = getCurrentWindow();
    const unlisten = await window.onDragDropEvent((event) => {
      if (event.payload.type === "enter" || event.payload.type === "over") {
        draggingOver.value = true;
      } else if (event.payload.type === "leave") {
        draggingOver.value = false;
      } else if (event.payload.type === "drop") {
        draggingOver.value = false;
        const validPaths = event.payload.paths.filter(hasSupportedExtension);
        if (validPaths.length > 0) {
          void handleDroppedFiles(validPaths);
        }
      }
    });
    if (unmounted) {
      unlisten();
      return;
    }
    stopDragDropListener = unlisten;
  } catch {
    // drag-drop not supported in this environment
  }
}

async function handleDroppedFiles(paths: string[]) {
  const filePath = paths[0];
  if (!filePath) return;
  if (opening.value) return;
  opening.value = true;
  error.value = null;
  try {
    const result = await openDocumentByPath(filePath);
    await adoptDocument(result);
  } catch (cause) {
    if (!isCancelled(cause)) error.value = safeMessage(cause);
  } finally {
    opening.value = false;
  }
}

function renderCacheKey(
  pageNumber: number,
  dimensions: { width: number; height: number },
): string {
  const currentHandle = handle.value;
  if (!currentHandle) return "";
  return `${currentHandle.session}:${currentHandle.document}:${pageNumber}:${dimensions.width}:${dimensions.height}`;
}

async function preloadNextPage(pageNumber: number, currentHandle: DocumentHandleDto) {
  if (pageNumber >= pageCount.value) return;
  const dimensions = targetDimensions.value;
  if (!dimensions || dimensions.width <= 1 || dimensions.height <= 1) return;
  const key = renderCacheKey(pageNumber + 1, dimensions);
  if (preloadedPages.value.has(key)) return;
  try {
    const result = await renderPage(
      currentHandle,
      pageNumber,
      dimensions.width,
      dimensions.height,
    );
    if (unmounted || handle.value !== currentHandle) return;
    if (preloadedPages.value.size >= 3) preloadedPages.value.clear();
    preloadedPages.value.set(key, result);
  } catch {
    // Preloading is best-effort; a failure just means the page renders on demand.
  }
}

async function loadPage(pageNumber: number) {
  const currentHandle = handle.value;
  const dimensions = targetDimensions.value;
  if (!currentHandle || !info.value || !dimensions) return;
  if (dimensions.width <= 1 || dimensions.height <= 1) return;

  const cacheKey = renderCacheKey(pageNumber, dimensions);
  if (cacheKey === failedRenderKey) return;

  const cached = preloadedPages.value.get(cacheKey);
  if (cached) {
    preloadedPages.value.delete(cacheKey);
    renderSequence.value += 1;
    renderedPage.value = cached;
    renderedPageNumber.value = pageNumber;
    failedRenderKey = null;
    error.value = null;
    void preloadNextPage(pageNumber, currentHandle);
    return;
  }

  const sequence = ++renderSequence.value;
  rendering.value = true;
  try {
    const result = await renderPage(
      currentHandle,
      pageNumber - 1,
      dimensions.width,
      dimensions.height,
    );
    if (sequence === renderSequence.value) {
      renderedPage.value = result;
      renderedPageNumber.value = pageNumber;
      failedRenderKey = null;
      error.value = null;
      void preloadNextPage(pageNumber, currentHandle);
    }
  } catch (cause) {
    if (sequence !== renderSequence.value) return;
    failedRenderKey = cacheKey;
    error.value = safeMessage(cause);
    renderedPage.value = null;
  } finally {
    if (sequence === renderSequence.value) rendering.value = false;
  }
}

watch(
  [handle, info, targetDimensions, page],
  (current, previous) => {
    if (!hasDocument.value || viewportWidth.value === 0) return;
    renderSequence.value += 1;
    rendering.value = false;
    const pageChanged = current[3] !== previous?.[3];
    if (pageChanged) {
      const viewport = viewportElement.value;
      if (viewport) viewport.scrollTop = 0;
    }
    const timer = window.setTimeout(() => void loadPage(page.value), pageChanged ? 0 : 100);
    onWatcherCleanup(() => window.clearTimeout(timer));
  },
  { flush: "post" },
);

async function handleOpen() {
  opening.value = true;
  error.value = null;
  let newHandle: DocumentHandleDto | null = null;

  try {
    const result = await openDocument();
    newHandle = result.handle;
    await adoptDocument(result);
  } catch (cause) {
    if (newHandle) void closeDocument(newHandle).catch(() => undefined);
    if (!isCancelled(cause)) error.value = safeMessage(cause);
  } finally {
    opening.value = false;
    void drainStartupDocuments();
  }
}

async function drainStartupDocuments() {
  startupDrainRequested = true;
  if (drainingStartupDocuments || opening.value) return;
  drainingStartupDocuments = true;
  opening.value = true;
  try {
    let consecutiveFailures = 0;
    while (startupDrainRequested) {
      startupDrainRequested = false;
      while (true) {
        let result;
        try {
          result = await openStartupDocument();
        } catch (cause) {
          error.value = safeMessage(cause);
          consecutiveFailures += 1;
          if (consecutiveFailures >= 4) return;
          continue;
        }
        if (!result) break;
        consecutiveFailures = 0;
        try {
          await adoptDocument(result);
          error.value = null;
        } catch (cause) {
          void closeDocument(result.handle).catch(() => undefined);
          error.value = safeMessage(cause);
        }
      }
    }
  } finally {
    opening.value = false;
    drainingStartupDocuments = false;
  }
}

async function adoptDocument(result: { handle: DocumentHandleDto; file_name: string }) {
  const documentInfo = await getDocumentInfo(result.handle);
  const previousHandle = handle.value;
  renderSequence.value += 1;
  failedRenderKey = null;
  preloadedPages.value = new Map();
  handle.value = result.handle;
  info.value = documentInfo;
  fileLabel.value = result.file_name;
  page.value = 1;
  fitMode.value = "fitWidth";
  zoom.value = 1;
  renderedPage.value = null;
  renderedPageNumber.value = 1;
  if (previousHandle) void closeDocument(previousHandle).catch(() => undefined);
}

async function handleClose() {
  if (!handle.value) return;
  renderSequence.value += 1;
  failedRenderKey = null;
  preloadedPages.value = new Map();
  const currentHandle = handle.value;
  handle.value = null;
  info.value = null;
  fileLabel.value = null;
  page.value = 1;
  renderedPage.value = null;
  renderedPageNumber.value = 1;
  rendering.value = false;
  error.value = null;

  try {
    await closeDocument(currentHandle);
  } catch (cause) {
    error.value = safeMessage(cause);
  }
}

function previousPage() {
  page.value = Math.max(1, page.value - 1);
}

function nextPage() {
  page.value = Math.min(pageCount.value || 1, page.value + 1);
}

function zoomIn() {
  zoom.value = Math.min(MAX_ZOOM, zoom.value * 1.25);
}

function zoomOut() {
  zoom.value = Math.max(MIN_ZOOM, zoom.value / 1.25);
}

function setFit(mode: FitMode) {
  fitMode.value = mode;
  zoom.value = 1;
}

function setZoomFromInput(event: Event) {
  zoom.value = Number((event.currentTarget as HTMLInputElement).value) / 100;
}

function onViewerWheel(event: WheelEvent) {
  if (!hasDocument.value) return;
  if (event.ctrlKey) {
    event.preventDefault();
    const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1;
    zoom.value = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom.value * factor));
    return;
  }
  const viewport = viewportElement.value;
  if (!viewport) return;
  const atTop = viewport.scrollTop <= 0;
  const atBottom = viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - 2;
  if (event.deltaY > 0 && atBottom) {
    event.preventDefault();
    nextPage();
  } else if (event.deltaY < 0 && atTop) {
    event.preventDefault();
    previousPage();
  }
}

function onViewerDoubleClick() {
  if (!hasDocument.value) return;
  zoom.value = zoom.value > 1 ? 1 : 2;
}

function startPan(event: PointerEvent) {
  const viewport = viewportElement.value;
  if (!viewport || !canPan.value || event.button !== 0) return;
  event.preventDefault();
  viewport.setPointerCapture(event.pointerId);
  panStart = {
    pointerId: event.pointerId,
    x: event.clientX,
    y: event.clientY,
    scrollLeft: viewport.scrollLeft,
    scrollTop: viewport.scrollTop,
  };
  panning.value = true;
}

function movePan(event: PointerEvent) {
  const viewport = viewportElement.value;
  if (!viewport || !panStart || event.pointerId !== panStart.pointerId) return;
  viewport.scrollLeft = panStart.scrollLeft - (event.clientX - panStart.x);
  viewport.scrollTop = panStart.scrollTop - (event.clientY - panStart.y);
}

function stopPan(event: PointerEvent) {
  const viewport = viewportElement.value;
  if (!panStart || event.pointerId !== panStart.pointerId) return;
  if (viewport?.hasPointerCapture(event.pointerId)) {
    viewport.releasePointerCapture(event.pointerId);
  }
  panStart = null;
  panning.value = false;
}

function setPageFromInput(event: Event) {
  const value = Number((event.currentTarget as HTMLInputElement).value);
  if (Number.isFinite(value)) {
    page.value = Math.min(pageCount.value, Math.max(1, value));
  }
}

function selectInput(event: FocusEvent) {
  (event.currentTarget as HTMLInputElement).select();
}

function onKeyDown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null;
  if (target?.matches("input, textarea, select")) return;

  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "o") {
    event.preventDefault();
    if (!opening.value && backendAvailable.value !== false) void handleOpen();
    return;
  }
  if (!hasDocument.value) return;

  if (event.key === "ArrowLeft" || event.key === "PageUp") previousPage();
  if (event.key === "ArrowRight" || event.key === "PageDown") nextPage();
  if (event.key === "Home") page.value = 1;
  if (event.key === "End") page.value = pageCount.value;
  if (event.key === "+" || event.key === "=") zoomIn();
  if (event.key === "-") zoomOut();
  if (event.key === "0") zoom.value = 1;
}
</script>

<template>
  <div class="app-shell">
    <header class="topbar">
      <div class="brand" aria-label="DocBunker">
        <span class="brand-mark" aria-hidden="true">
          <svg class="brand-glyph" viewBox="0 0 24 24">
            <rect class="sheet" x="5" y="4" width="10" height="13.5" rx="1.6" />
            <path class="fold" d="M15 7.6h-3.4V4.2" />
            <path class="lines" d="M7.8 9h5.6M7.8 11.4h5.6M7.8 13.8h3.4" />
            <path class="shackle" d="M9.6 15v-1.8a2.4 2.4 0 0 1 4.8 0V15" />
            <rect class="lockbody" x="8.2" y="14.9" width="7.6" height="5.4" rx="1.4" />
          </svg>
        </span>
        <span>DocBunker</span>
      </div>

      <div class="document-identity">
        <strong :title="fileLabel ?? undefined">{{ fileLabel ?? "Visor seguro" }}</strong>
        <span>{{ documentMeta }}</span>
      </div>

      <div class="topbar-actions">
        <button
          v-if="hasDocument"
          class="icon-button"
          type="button"
          :class="{ active: showThumbnails }"
          :aria-pressed="showThumbnails"
          aria-label="Miniaturas"
          title="Mostrar miniaturas"
          @click="showThumbnails = !showThumbnails"
        >
          <AppIcon name="grid" />
        </button>
        <button v-if="hasDocument" class="button button-quiet" type="button" @click="handleClose">
          <AppIcon name="close" />
          <span class="button-label">Cerrar</span>
        </button>
        <button
          class="button button-primary"
          type="button"
          :disabled="opening || backendAvailable === false"
          aria-keyshortcuts="Control+O Meta+O"
          @click="handleOpen"
        >
          <AppIcon name="open" />
          {{ opening ? "Abriendo…" : hasDocument ? "Abrir otro" : "Abrir documento" }}
        </button>
      </div>
    </header>

    <div v-if="error" class="error-banner" role="alert">
      <span>{{ error }}</span>
      <button type="button" aria-label="Cerrar aviso" @click="error = null">×</button>
    </div>

    <div class="content-area">
      <PageThumbnails
        v-if="hasDocument && showThumbnails"
        :handle="handle!"
        :info="info!"
        :current-page="page"
        @select:page="page = $event"
      />

      <main
        ref="viewportElement"
        class="viewer"
        :class="{
          'has-document': hasDocument,
          'is-empty': !hasDocument,
          'can-pan': canPan,
          'is-panning': panning,
        }"
        @wheel="onViewerWheel"
        @dblclick="onViewerDoubleClick"
        @pointerdown="startPan"
        @pointermove="movePan"
        @pointerup="stopPan"
        @pointercancel="stopPan"
      >
        <section v-if="!hasDocument" class="empty-state" aria-labelledby="empty-title">
          <div class="empty-mark" aria-hidden="true">
            <svg viewBox="0 0 84 84">
              <rect x="4" y="4" width="76" height="76" rx="14" fill="#212529" />
              <path
                d="M26 22h18l12 14v26H26z"
                fill="#343a40"
                stroke="#495057"
                stroke-width="2"
                stroke-linejoin="round"
              />
              <path
                d="M44 22v14h12"
                fill="#343a40"
                stroke="#495057"
                stroke-width="2"
                stroke-linejoin="round"
              />
              <path
                d="M34 42h20M34 50h20M34 58h12"
                stroke="#6c757d"
                stroke-width="3"
                stroke-linecap="round"
              />
              <circle cx="58" cy="60" r="8" fill="#0d6efd" />
              <circle cx="58" cy="60" r="3" fill="#cfe2ff" />
              <circle cx="20" cy="28" r="2.5" fill="#6ea8fe" />
            </svg>
          </div>
          <h1 id="empty-title">Abra un documento</h1>
          <p class="empty-copy">{{ emptyStateCopy }}</p>
          <button
            class="button button-primary button-large"
            type="button"
            :disabled="opening || backendAvailable === false"
            @click="handleOpen"
          >
            <AppIcon name="open" />
            {{ opening ? "Abriendo…" : "Seleccionar archivo…" }}
          </button>
          <div v-if="backendIsolated" class="trust-line">
            <span><AppIcon name="wifi-off" /> Sin acceso a Internet</span>
            <span><AppIcon name="clock" /> Sesión temporal</span>
            <span><AppIcon name="lock" /> Solo lectura</span>
          </div>
          <div class="shortcut-hint" aria-hidden="true">
            <AppIcon name="open" />
            <kbd>Ctrl</kbd>
            <span>+</span>
            <kbd>O</kbd>
            <span>para abrir</span>
          </div>
        </section>

        <div v-if="hasDocument" class="page-stage">
          <img
            v-if="renderedPage"
            class="rendered-page"
            :class="{ 'is-updating': rendering }"
            :src="renderedPage.data_url"
            :alt="`Página ${renderedPageNumber} de ${pageCount}`"
            :style="{
              width: renderedPage
                ? `${renderedPage.width / (DPR * RENDER_QUALITY)}px`
                : undefined,
            }"
            draggable="false"
          />
          <div v-else-if="rendering" class="page-skeleton" aria-hidden="true" />
        </div>

        <div v-if="rendering" class="render-status" role="status" aria-live="polite">
          <span class="spinner" /> Renderizando página {{ page }}
        </div>

        <div v-if="draggingOver" class="drag-overlay" aria-hidden="true">
          <div class="drag-overlay-content">
            <AppIcon name="open" />
            <span>Soltar archivo aquí</span>
          </div>
        </div>
      </main>
    </div>

    <footer class="controlbar" aria-label="Controles del documento">
      <div class="control-group page-controls">
        <button
          class="icon-button"
          type="button"
          :disabled="!hasDocument || page <= 1"
          :tabindex="!hasDocument || page <= 1 ? -1 : undefined"
          aria-label="Página anterior"
          aria-keyshortcuts="ArrowLeft PageUp"
          @click="previousPage"
        >
          <AppIcon name="left" />
        </button>
        <label class="page-field">
          <span class="sr-only">Página actual</span>
          <input
            type="number"
            min="1"
            :max="Math.max(1, pageCount)"
            :value="hasDocument ? page : ''"
            :disabled="!hasDocument"
            @input="setPageFromInput"
            @focus="selectInput"
          />
          <span v-if="hasDocument">/ {{ pageCount }}</span>
          <span v-else> — </span>
        </label>
        <button
          class="icon-button"
          type="button"
          :disabled="!hasDocument || page >= pageCount"
          :tabindex="!hasDocument || page >= pageCount ? -1 : undefined"
          aria-label="Página siguiente"
          aria-keyshortcuts="ArrowRight PageDown"
          @click="nextPage"
        >
          <AppIcon name="right" />
        </button>
      </div>

      <div
        class="sandbox-status"
        :class="{ 'is-unisolated': backendAvailable === false || backendIsolated === false }"
        :title="
          backendAvailable === false
            ? 'No se pudo iniciar el servicio de documentos'
            : backendIsolated
            ? 'El documento se procesa en un entorno aislado'
            : 'El backend activo no proporciona aislamiento'
        "
      >
        <span class="status-dot" />
        <span>{{ isolationLabel }}</span>
      </div>

      <div class="control-group view-controls">
        <div class="segmented" aria-label="Modo de ajuste">
          <button
            type="button"
            :class="{ active: fitMode === 'fitWidth' }"
            :disabled="!hasDocument"
            :aria-pressed="fitMode === 'fitWidth'"
            @click="setFit('fitWidth')"
          >
            Ancho
          </button>
          <button
            type="button"
            :class="{ active: fitMode === 'fitPage' }"
            :disabled="!hasDocument"
            :aria-pressed="fitMode === 'fitPage'"
            @click="setFit('fitPage')"
          >
            Página
          </button>
        </div>
        <span class="control-divider" />
        <button
          class="icon-button"
          type="button"
          :disabled="!hasDocument || zoom <= MIN_ZOOM"
          aria-label="Alejar"
          aria-keyshortcuts="-"
          @click="zoomOut"
        >
          <AppIcon name="minus" />
        </button>
        <label class="zoom-slider" title="Nivel de zoom">
          <span class="sr-only">Nivel de zoom</span>
          <input
            type="range"
            :min="MIN_ZOOM * 100"
            :max="MAX_ZOOM * 100"
            step="5"
            :value="zoom * 100"
            :disabled="!hasDocument"
            @input="setZoomFromInput"
          />
        </label>
        <button
          class="zoom-value"
          type="button"
          :disabled="!hasDocument"
          title="Restablecer zoom"
          aria-keyshortcuts="0"
          @click="zoom = 1"
        >
          {{ Math.round(zoom * 100) }}%
        </button>
        <button
          class="icon-button"
          type="button"
          :disabled="!hasDocument || zoom >= MAX_ZOOM"
          aria-label="Acercar"
          aria-keyshortcuts="+"
          @click="zoomIn"
        >
          <AppIcon name="plus" />
        </button>
      </div>
    </footer>
  </div>
</template>
