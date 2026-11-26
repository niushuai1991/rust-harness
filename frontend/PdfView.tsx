import { useEffect, useRef, useState } from "react";
import * as pdfjs from "pdfjs-dist/legacy/build/pdf.mjs";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import type { PDFDocumentProxy } from "pdfjs-dist/types/src/display/api";
import { command, errorText } from "./api";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

export function PdfView({ path, sessionId = "" }: { path: string; sessionId?: string }) {
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null),
    [page, setPage] = useState(1),
    [scale, setScale] = useState(1),
    [error, setError] = useState("");
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    let alive = true;
    let task: ReturnType<typeof pdfjs.getDocument> | undefined;
    setDocument(null);
    setPage(1);
    setScale(1);
    setError("");
    void command<number[]>("read_local_pdf", { path, sessionId })
      .then((bytes) => {
        if (!alive) return;
        task = pdfjs.getDocument({
          data: new Uint8Array(bytes),
          cMapUrl: `${import.meta.env.BASE_URL}pdfjs/cmaps/`,
          cMapPacked: true,
          standardFontDataUrl: `${import.meta.env.BASE_URL}pdfjs/standard_fonts/`,
        });
        return task.promise;
      })
      .then((doc) => {
        if (alive && doc) setDocument(doc);
      })
      .catch((e) => {
        if (alive) setError(errorText(e));
      });
    return () => {
      alive = false;
      void task?.destroy();
    };
  }, [path, sessionId]);
  useEffect(() => {
    if (!document) return;
    let alive = true;
    let renderTask: { cancel: () => void; promise: Promise<void> } | undefined;
    void document
      .getPage(page)
      .then((pdfPage) => {
        if (!alive || !canvas.current) return;
        const viewport = pdfPage.getViewport({ scale });
        const element = canvas.current;
        const context = element.getContext("2d");
        if (!context) throw new Error("无法创建 PDF 画布");
        element.width = Math.ceil(viewport.width);
        element.height = Math.ceil(viewport.height);
        renderTask = pdfPage.render({ canvasContext: context, viewport });
        return renderTask.promise;
      })
      .catch((e) => {
        if (alive) setError(errorText(e));
      });
    return () => {
      alive = false;
      renderTask?.cancel();
    };
  }, [document, page, scale]);
  return (
    <div className="pdf-view">
      <div className="preview-head">
        <span title={path}>{path}</span>
      </div>
      {error ? (
        <div className="side-empty">
          <p role="alert" className="inline-error">
            {error}
          </p>
        </div>
      ) : !document ? (
        <div className="side-empty">正在加载 PDF…</div>
      ) : (
        <div className="pdf-page">
          <canvas key={`${page}-${scale}`} ref={canvas} aria-label={`PDF 第 ${page} 页`} />
        </div>
      )}
      {document && (
        <div className="pdf-controls">
          <button
            type="button"
            aria-label="上一页"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            上一页
          </button>
          <span>
            {page} / {document.numPages}
          </span>
          <button
            type="button"
            aria-label="下一页"
            disabled={page >= document.numPages}
            onClick={() => setPage((p) => p + 1)}
          >
            下一页
          </button>
          <button
            type="button"
            aria-label="缩小"
            disabled={scale <= 0.25}
            onClick={() => setScale((s) => Math.max(0.25, s - 0.25))}
          >
            −
          </button>
          <span>{Math.round(scale * 100)}%</span>
          <button
            type="button"
            aria-label="放大"
            disabled={scale >= 4}
            onClick={() => setScale((s) => Math.min(4, s + 0.25))}
          >
            ＋
          </button>
        </div>
      )}
    </div>
  );
}
