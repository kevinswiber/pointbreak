// Inline + block markdown to HTML. Ported from the served app.js markdown
// cluster. Imports escape and refs; the import direction is markdown -> refs ->
// escape (no cycle).

import { bodyClass } from "./classNames";
import { escapeHtml } from "./escape";
import {
  isMarkdownContentType,
  linkify,
  linkifyEscaped,
  safeMarkdownHref,
} from "./refs";

/** Wrap rendered body content in a div, marking markdown bodies; "" for empty text. */
export function renderBodyContent(
  text: unknown,
  contentType: string | undefined,
): string {
  if (!text) return "";
  const cls = bodyClass("anno-body", isMarkdownContentType(contentType));
  return `<div class="${cls}">${renderContentHtml(text, contentType)}</div>`;
}

/** Render body content as markdown when the content type selects it, else linkified text. */
export function renderContentHtml(
  text: unknown,
  contentType: string | undefined,
): string {
  return isMarkdownContentType(contentType)
    ? renderMarkdown(text)
    : linkify(text);
}

/** Render block-level markdown (headings, paragraphs, lists, fenced code) to HTML. */
export function renderMarkdown(text: unknown): string {
  const lines = String(text ?? "")
    .replace(/\r\n?/g, "\n")
    .split("\n");
  const out: string[] = [];
  let paragraph: string[] = [];
  let listKind: "ul" | "ol" | null = null;
  let listItems: string[] = [];

  const flushParagraph = (): void => {
    if (!paragraph.length) return;
    out.push(`<p>${renderMarkdownInline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };
  const flushList = (): void => {
    if (!listKind) return;
    out.push(
      `<${listKind}>${listItems.map((item) => `<li>${renderMarkdownInline(item)}</li>`).join("")}</${listKind}>`,
    );
    listKind = null;
    listItems = [];
  };
  const flushBlocks = (): void => {
    flushParagraph();
    flushList();
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = line.match(/^\s*```/);
    if (fence) {
      flushBlocks();
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      out.push(`<pre><code>${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }
    if (!line.trim()) {
      flushBlocks();
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      flushBlocks();
      const level = heading[1].length;
      out.push(
        `<h${level}>${renderMarkdownInline(heading[2].trim())}</h${level}>`,
      );
      continue;
    }
    const unordered = line.match(/^\s*[-*]\s+(.+)$/);
    if (unordered) {
      flushParagraph();
      if (listKind && listKind !== "ul") flushList();
      listKind = "ul";
      listItems.push(unordered[1]);
      continue;
    }
    const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/);
    if (ordered) {
      flushParagraph();
      if (listKind && listKind !== "ol") flushList();
      listKind = "ol";
      listItems.push(ordered[1]);
      continue;
    }
    if (listKind) flushList();
    paragraph.push(line.trim());
  }
  flushBlocks();
  return out.join("");
}

/** Render inline markdown (code, links, emphasis) to HTML, escaping user content. */
export function renderMarkdownInline(text: unknown): string {
  const placeholders: Array<[string, string]> = [];
  const stash = (html: string): string => {
    const token = `\u0000MD${placeholders.length}\u0000`;
    placeholders.push([token, html]);
    return token;
  };
  let html = escapeHtml(String(text ?? ""));
  html = html.replace(/`([^`]+)`/g, (_: string, code: string) =>
    stash(`<code>${code}</code>`),
  );
  html = html.replace(
    /\[([^\]]+)\]\(([^)\s]+)\)/g,
    (_: string, label: string, href: string) => {
      const safe = safeMarkdownHref(href);
      const labelHtml = renderMarkdownInline(label);
      return safe
        ? stash(
            `<a href="${safe}" target="_blank" rel="noreferrer">${labelHtml}</a>`,
          )
        : labelHtml;
    },
  );
  html = html
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>");
  html = linkifyEscaped(html);
  for (const [token, replacement] of placeholders) {
    html = html.split(token).join(replacement);
  }
  return html;
}
