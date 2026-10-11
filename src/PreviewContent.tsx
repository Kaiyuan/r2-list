import { useEffect, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import type { TranslationKey } from "./i18n";

type PreviewEntry = { name: string; extension: string; kind: string };
type SheetPreview = { name: string; rows: unknown[][] };
type ParsedOffice = { type: "html"; html: string } | { type: "sheets"; sheets: SheetPreview[] } | { type: "slides"; slides: string[][] } | null;

type Props = {
  entry: PreviewEntry;
  bytes: Uint8Array | null;
  objectUrl: string | null;
  loading: boolean;
  error: string;
  t: (key: TranslationKey) => string;
};

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
}

function parseText(bytes: Uint8Array, extension: string): string {
  const text = new TextDecoder("utf-8").decode(bytes);
  if (extension === "json" || extension === "jsonl") {
    try { return JSON.stringify(JSON.parse(text), null, 2); }
    catch { return text; }
  }
  return text;
}

function parsePresentation(bytes: Uint8Array, unzipSync: typeof import("fflate").unzipSync, strFromU8: typeof import("fflate").strFromU8): string[][] {
  const files = unzipSync(bytes);
  const slideFiles = Object.keys(files)
    .filter((path) => /^ppt\/slides\/slide\d+\.xml$/.test(path))
    .sort((left, right) => Number(left.match(/slide(\d+)/)?.[1]) - Number(right.match(/slide(\d+)/)?.[1]));
  const namespace = "http://schemas.openxmlformats.org/drawingml/2006/main";
  return slideFiles.map((path) => {
    const xml = new DOMParser().parseFromString(strFromU8(files[path]), "application/xml");
    return Array.from(xml.getElementsByTagNameNS(namespace, "p"))
      .map((paragraph) => Array.from(paragraph.getElementsByTagNameNS(namespace, "t"))
        .map((part) => part.textContent ?? "").join(""))
      .filter(Boolean);
  });
}

export function PreviewContent({ entry, bytes, objectUrl, loading, error, t }: Props) {
  const docxContainer = useRef<HTMLDivElement>(null);
  const [text, setText] = useState("");
  const [office, setOffice] = useState<ParsedOffice>(null);
  const [parseError, setParseError] = useState("");

  useEffect(() => {
    let active = true;
    setText("");
    setOffice(null);
    setParseError("");
    docxContainer.current?.replaceChildren();
    if (!bytes) return;

    try {
      if (entry.kind === "text") {
        const content = parseText(bytes, entry.extension);
        if (entry.extension === "md" || entry.extension === "markdown") {
          const html = marked.parse(content, { async: false }) as string;
          setOffice({ type: "html", html: DOMPurify.sanitize(html) });
        } else if (entry.extension === "csv" || entry.extension === "tsv") {
          void import("papaparse").then(({ default: Papa }) => {
            const result = Papa.parse<string[]>(content, { delimiter: entry.extension === "tsv" ? "\t" : ",", skipEmptyLines: true, preview: 200 });
            if (result.errors.length) throw new Error(result.errors[0].message);
            if (active) setOffice({ type: "sheets", sheets: [{ name: entry.name, rows: result.data }] });
          }).catch((reason: unknown) => { if (active) setParseError(String(reason)); });
        } else {
          setText(content);
        }
      } else if (entry.extension === "docx") {
        void import("docx-preview").then(async ({ renderAsync }) => {
          const container = docxContainer.current;
          if (active && container) await renderAsync(toArrayBuffer(bytes), container);
        }).catch((reason: unknown) => { if (active) setParseError(String(reason)); });
      } else if (entry.extension === "xlsx") {
        void import("exceljs").then(async ({ default: ExcelJS }) => {
          const workbook = new ExcelJS.Workbook();
          await workbook.xlsx.load(toArrayBuffer(bytes));
          const sheets = workbook.worksheets.map((worksheet) => {
            const rows: unknown[][] = [];
            worksheet.eachRow((row) => {
              if (rows.length >= 200) return;
              const values: unknown[] = [];
              row.eachCell({ includeEmpty: true }, (cell, column) => {
                const value: unknown = cell.value;
                if (value instanceof Date) values[column - 1] = value.toLocaleString();
                else if (value && typeof value === "object") {
                  if ("text" in value) values[column - 1] = value.text;
                  else if ("result" in value) values[column - 1] = value.result;
                  else values[column - 1] = JSON.stringify(value);
                } else {
                  values[column - 1] = value ?? "";
                }
              });
              rows.push(values);
            });
            return { name: worksheet.name, rows };
          });
          if (active) setOffice({ type: "sheets", sheets });
        }).catch((reason: unknown) => { if (active) setParseError(String(reason)); });
      } else if (entry.extension === "pptx") {
        void import("fflate").then(({ unzipSync, strFromU8 }) => {
          if (active) setOffice({ type: "slides", slides: parsePresentation(bytes, unzipSync, strFromU8) });
        }).catch((reason: unknown) => { if (active) setParseError(String(reason)); });
      }
    } catch (reason) {
      setParseError(String(reason));
    }
    return () => { active = false; };
  }, [bytes, entry.extension, entry.kind]);

  if (loading) return <div className="file-preview-message">{t("loadingPreview")}</div>;
  if (error) return <div className="file-preview-message file-preview-error">{error}</div>;
  if (entry.kind === "image" && objectUrl) return <img className="file-preview-image" src={objectUrl} alt={entry.name} />;
  if (entry.kind === "video" && objectUrl) return <video className="file-preview-media" src={objectUrl} controls preload="metadata" />;
  if (entry.kind === "audio" && objectUrl) return <audio className="file-preview-audio" src={objectUrl} controls preload="metadata" />;
  if (entry.extension === "pdf" && objectUrl) return <iframe className="file-preview-pdf" src={objectUrl} title={entry.name} />;
  if (entry.extension === "docx") return <div className="file-preview-docx" ref={docxContainer} />;
  if (!bytes) return <div className="file-preview-message">{t("previewPending")}</div>;
  if (parseError) return <div className="file-preview-message file-preview-error">{parseError}</div>;

  if (office?.type === "html") return <article className="file-preview-rich-text" dangerouslySetInnerHTML={{ __html: office.html }} />;
  if (office?.type === "sheets") return <div className="file-preview-sheets">{office.sheets.map((sheet) => <section key={sheet.name}><h3>{sheet.name}</h3><table><tbody>{sheet.rows.map((row, rowIndex) => <tr key={rowIndex}>{row.map((cell, cellIndex) => <td key={cellIndex}>{cell instanceof Date ? cell.toLocaleString() : String(cell ?? "")}</td>)}</tr>)}</tbody></table></section>)}</div>;
  if (office?.type === "slides") return <div className="file-preview-slides">{office.slides.map((slide, index) => <article key={index}><span>SLIDE {index + 1}</span>{slide.map((line, lineIndex) => <p key={lineIndex}>{line}</p>)}</article>)}</div>;
  if (entry.kind === "text") return <pre className="file-preview-plain-text">{text}</pre>;
  return <div className="file-preview-message">{t("previewPending")}</div>;
}