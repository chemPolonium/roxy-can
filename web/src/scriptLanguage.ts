import {
  HighlightStyle,
  StreamLanguage,
  foldService,
  indentService,
  indentUnit,
} from "@codemirror/language";
import { tags } from "@lezer/highlight";
import { SCRIPT_KEYWORDS } from "./scriptKeywords";

interface ScriptState {
  blockComment: boolean;
}

/// The `.rxcan` language: a small C-like tokenizer plus the brace fold and
/// indentation rules the scripts use. The completion engine stays outside the
/// mode; only the lexical state lives here.
export const scriptLanguage = StreamLanguage.define<ScriptState>({
  name: "rxcan",
  startState: () => ({ blockComment: false }),
  copyState: (state) => ({ ...state }),
  tokenTable: {
    function: tags.function(tags.variableName),
    bracket: tags.bracket,
  },
  languageData: {
    commentTokens: {
      line: "//",
      block: { open: "/*", close: "*/" },
    },
  },
  token(stream, state) {
    if (state.blockComment) {
      if (stream.skipTo("*/")) {
        stream.match("*/");
        state.blockComment = false;
      } else {
        stream.skipToEnd();
      }
      return "comment";
    }
    if (stream.eatSpace()) return null;
    if (stream.match("//")) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match("/*")) {
      state.blockComment = true;
      if (stream.skipTo("*/")) {
        stream.match("*/");
        state.blockComment = false;
      } else {
        stream.skipToEnd();
      }
      return "comment";
    }
    const quote = stream.peek();
    if (quote === '"' || quote === "'") {
      stream.next();
      let escaped = false;
      while (!stream.eol()) {
        const ch = stream.next();
        if (escaped) {
          escaped = false;
        } else if (ch === "\\") {
          escaped = true;
        } else if (ch === quote) {
          break;
        }
      }
      return "string";
    }
    if (stream.match(/0[xX][0-9a-fA-F]+|\d+(?:\.\d+)?/)) return "number";
    if (stream.match(/[A-Za-z_]\w*/)) {
      const word = stream.current();
      if (SCRIPT_KEYWORDS.has(word)) return "keyword";
      if (stream.peek() === "(") return "function";
      return "variableName";
    }
    if (stream.match(/[()[\]{}]/)) return "bracket";
    if (stream.match(/[+\-*/%=<>!&|^~?:.,;]/)) return "operator";
    stream.next();
    return null;
  },
});

/// Four spaces per level, one extra level after an opening bracket, one less
/// before a closing one. This mirrors the old editor's Enter/Tab behaviour.
export const scriptIndent = indentService.of((context, pos) => {
  const doc = context.state.doc;
  const line = doc.lineAt(pos);
  if (line.number <= 1) return 0;
  const previous = doc.line(line.number - 1);
  const indent = (text: string) => {
    const match = /^[ \t]*/.exec(text);
    return (match?.[0].length ?? 0) * (match?.[0].includes("\t") ? context.unit : 1);
  };
  let width = indent(previous.text);
  if (/[{([]\s*$/.test(previous.text.trimEnd())) width += context.unit;
  if (/^\s*[}\])]/.test(line.text)) width -= context.unit;
  return Math.max(0, width);
});

/// Find the `}` that closes the `{` at `open`, skipping comments and strings.
function matchingBrace(doc: string, open: number): number | null {
  let depth = 0;
  let quote: string | null = null;
  let escape = false;
  let lineComment = false;
  let blockComment = false;
  for (let i = open; i < doc.length; i += 1) {
    const ch = doc[i];
    const next = doc[i + 1];
    if (lineComment) {
      if (ch === "\n") lineComment = false;
      continue;
    }
    if (blockComment) {
      if (ch === "*" && next === "/") {
        blockComment = false;
        i += 1;
      }
      continue;
    }
    if (quote) {
      if (escape) {
        escape = false;
      } else if (ch === "\\") {
        escape = true;
      } else if (ch === quote) {
        quote = null;
      }
      continue;
    }
    if (ch === "/" && next === "/") {
      lineComment = true;
      i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      blockComment = true;
      i += 1;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      continue;
    }
    if (ch === "{") depth += 1;
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return null;
}

/// Fold a line whose visible end is `{`, hiding everything through its brace.
export const scriptFold = foldService.of((state, lineStart) => {
  const line = state.doc.lineAt(lineStart);
  const open = line.text.lastIndexOf("{");
  if (open < 0 || line.text.slice(open + 1).trim() !== "") return null;
  const close = matchingBrace(state.doc.toString(), line.from + open);
  if (close == null || close <= line.to) return null;
  return { from: line.to, to: close };
});

export type EditorTheme = "dark" | "light";

export function scriptHighlightStyle(theme: EditorTheme) {
  if (theme === "light") {
    return HighlightStyle.define([
      { tag: tags.comment, color: "#7a8796" },
      { tag: tags.string, color: "#1f7a52" },
      { tag: tags.number, color: "#9a5b00" },
      { tag: tags.keyword, color: "#1f5fae" },
      { tag: tags.function(tags.variableName), color: "#7b3fb0" },
      { tag: tags.variableName, color: "#26313f" },
      { tag: tags.operator, color: "#5b6b7c" },
      { tag: tags.bracket, color: "#5b6b7c" },
    ]);
  }
  return HighlightStyle.define([
    { tag: tags.comment, color: "#6a7a8c" },
    { tag: tags.string, color: "#8fd3a8" },
    { tag: tags.number, color: "#e6b455" },
    { tag: tags.keyword, color: "#6cb6ff" },
    { tag: tags.function(tags.variableName), color: "#c77dff" },
    { tag: tags.variableName, color: "#d6e2f0" },
    { tag: tags.operator, color: "#93a4b8" },
    { tag: tags.bracket, color: "#93a4b8" },
  ]);
}

export const scriptIndentUnit = indentUnit.of("    ");
