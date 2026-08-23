import { invoke } from "@tauri-apps/api/core";
import type {
  BackendStatusDto,
  DocumentHandleDto,
  DocumentInfoDto,
  OpenResultDto,
  RenderedPageDto,
  DocBunkerErrorPayload,
} from "./types";

export function getBackendStatus(): Promise<BackendStatusDto> {
  return invoke<BackendStatusDto>("get_backend_status");
}

/** Parse the payload Tauri rejects with (stringified JSON or object). */
export function readError(e: unknown): DocBunkerErrorPayload | null {
  if (e && typeof e === "object" && "code" in e) {
    const candidate = e as DocBunkerErrorPayload;
    if (typeof candidate.code === "string") return candidate;
  }
  if (typeof e === "string") {
    try {
      const parsed = JSON.parse(e) as DocBunkerErrorPayload;
      if (parsed && typeof parsed.code === "string") return parsed;
    } catch {
      /* not JSON */
    }
    return { code: "unknown", message: e };
  }
  return null;
}

export function isCancelled(e: unknown): boolean {
  return readError(e)?.code === "cancelled";
}

/** A user-safe message; never raw parser output (DocBunkerError contract). */
export function safeMessage(e: unknown): string {
  return readError(e)?.message ?? "Something went wrong.";
}

export function openDocument(): Promise<OpenResultDto> {
  return invoke<OpenResultDto>("open_document");
}

export function openDocumentByPath(path: string): Promise<OpenResultDto> {
  return invoke<OpenResultDto>("open_document_by_path", { path });
}

export function openStartupDocument(): Promise<OpenResultDto | null> {
  return invoke<OpenResultDto | null>("open_startup_document");
}

export function getDocumentInfo(handle: DocumentHandleDto): Promise<DocumentInfoDto> {
  return invoke<DocumentInfoDto>("get_document_info", { handle });
}

export function renderPage(
  handle: DocumentHandleDto,
  page: number,
  targetWidth: number,
  targetHeight: number,
): Promise<RenderedPageDto> {
  return invoke<RenderedPageDto>("render_page", { handle, page, targetWidth, targetHeight });
}

export function closeDocument(handle: DocumentHandleDto): Promise<void> {
  return invoke<void>("close_document", { handle });
}
