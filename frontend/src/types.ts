export interface DocumentHandleDto {
  session: number;
  document: number;
}

export interface BackendStatusDto {
  available: boolean;
  isolated: boolean;
}

export interface OpenResultDto {
  handle: DocumentHandleDto;
  file_name: string;
}

export interface DocumentInfoDto {
  page_count: number;
  width: number;
  height: number;
  format: string;
}

export interface RenderedPageDto {
  width: number;
  height: number;
  data_url: string;
}

export interface DocBunkerErrorPayload {
  code: string;
  message: string;
}
