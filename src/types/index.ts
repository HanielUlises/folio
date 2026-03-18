export interface Topic {
  id: string;
  name: string;
  color: string;
}

export interface PdfEntry {
  id: string;
  path: string;
  name: string;
  size: number;
  added: number;
  topicId: string | null;
  exists: boolean;
}

export interface HighlightRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PdfHighlight {
  id: string;
  page: number;
  rects: HighlightRect[];
  color: string;
}

export interface AppData {
  topics: Topic[];
  pdfs: PdfEntry[];
  highlights: Record<string, PdfHighlight[]>;  
}

export interface OpenedFile {
  path: string;
  name: string;
  size: number;
  added: number;
}


export interface IpcChannels {
  'get-data':        { args: [];                 result: AppData      };
  'save-data':       { args: [AppData];          result: true         };
  'check-exists':    { args: [string];           result: boolean      };
  'open-pdf-dialog': { args: [];                 result: OpenedFile[] };
  'get-folio-url':   { args: [string];           result: string       };
}


export interface FolioApi {
  getData:       ()           => Promise<AppData>;
  saveData:      (d: AppData) => Promise<true>;
  checkExists:   (p: string)  => Promise<boolean>;
  openPdfDialog: ()           => Promise<OpenedFile[]>;
  getFolioUrl:   (p: string)  => Promise<string>;
}

declare global {
  interface Window {
    api: FolioApi;
  }
}
