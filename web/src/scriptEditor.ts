import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, startCompletion } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import { search, searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import {
  EditorState,
  RangeSetBuilder,
  StateEffect,
  StateField,
} from "@codemirror/state";
import {
  Decoration,
  DecorationSet,
  drawSelection,
  dropCursor,
  EditorView,
  GutterMarker,
  gutter,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  placeholder,
  type KeyBinding,
} from "@codemirror/view";
import {
  scriptFold,
  scriptHighlightStyle,
  scriptIndent,
  scriptIndentUnit,
  scriptLanguage,
} from "./scriptLanguage";

export interface ScriptCompletionItem {
  label: string;
  detail: string;
  insert: string;
  caret: number;
}

export interface ScriptCompletionResult {
  start: number;
  end: number;
  items: ScriptCompletionItem[];
}

export interface ScriptEditorCallbacks {
  completion: (
    doc: string,
    position: number,
    explicit: boolean,
  ) => ScriptCompletionResult | null;
  onChange: (doc: string) => void;
  onToggleBreakpoint: (line: number) => void;
  onCursor?: (
    doc: string,
    position: number,
    coords: { left: number; top: number } | null,
  ) => void;
}

export interface ScriptEditorApi {
  setDoc: (doc: string, reset: boolean) => void;
  setBreakpoints: (lines: number[]) => void;
  setDebugLine: (line: number | null) => void;
  setIssueLine: (line: number | null) => void;
  revealLine: (line: number, column?: number) => void;
  focus: () => void;
  destroy: () => void;
}

const setBreakpoints = StateEffect.define<number[]>();
const setDebugLine = StateEffect.define<number | null>();
const setIssueLine = StateEffect.define<number | null>();

class BreakpointMarker extends GutterMarker {
  toDOM(): Node {
    const dot = document.createElement("span");
    dot.className = "cm-bp";
    return dot;
  }
}

class BreakpointSpacer extends GutterMarker {
  toDOM(): Node {
    const spacer = document.createElement("span");
    spacer.className = "cm-bp-spacer";
    return spacer;
  }
}

const breakpointsField = StateField.define<ReturnType<typeof breakpointSet>>({
  create: () => breakpointSet([], null),
  update(value, tr) {
    let next = value.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setBreakpoints)) {
        next = breakpointSet(effect.value, tr.state);
      }
    }
    return next;
  },
});

function breakpointSet(lines: number[], state: EditorState | null) {
  const builder = new RangeSetBuilder<GutterMarker>();
  if (!state) return builder.finish();
  for (const lineNo of [...lines].sort((a, b) => a - b)) {
    if (lineNo < 1 || lineNo > state.doc.lines) continue;
    const from = state.doc.line(lineNo).from;
    builder.add(from, from, new BreakpointMarker());
  }
  return builder.finish();
}

function lineDecorations(effect: StateEffectTypeAlias, className: string) {
  return StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(value, tr) {
      let next = value.map(tr.changes);
      for (const entry of tr.effects) {
        if (entry.is(effect)) {
          const line = entry.value;
          if (line && line >= 1 && line <= tr.state.doc.lines) {
            const decoration = Decoration.line({ class: className });
            next = Decoration.set([decoration.range(tr.state.doc.line(line).from)]);
          } else {
            next = Decoration.none;
          }
        }
      }
      return next;
    },
  });
}

type StateEffectTypeAlias =
  | typeof setDebugLine
  | typeof setIssueLine;

const debugField = lineDecorations(setDebugLine, "cm-debug-line");
const issueField = lineDecorations(setIssueLine, "cm-issue-line");

const editorTheme = EditorView.theme(
  {
    "&": {
      height: "100%",
      backgroundColor: "transparent",
      color: "#d6e2f0",
    },
    ".cm-scroller": {
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: "11.5px",
      lineHeight: "1.45",
    },
    ".cm-content": {
      padding: "8px 0",
      caretColor: "#e8f0f8",
    },
    ".cm-line": {
      padding: "0 8px",
    },
    ".cm-gutters": {
      backgroundColor: "#0d141d",
      borderRight: "1px solid #263140",
      color: "#5d6a7a",
    },
    ".cm-activeLine": {
      backgroundColor: "rgba(76, 157, 255, 0.06)",
    },
    ".cm-activeLineGutter": {
      backgroundColor: "#111a25",
    },
    ".cm-cursor": {
      borderLeftColor: "#e8f0f8",
    },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection":
      {
        backgroundColor: "rgba(76, 157, 255, 0.35)",
      },
    ".cm-tooltip": {
      backgroundColor: "#0d141d",
      border: "1px solid #2d4055",
      color: "#cdd8e4",
    },
    ".cm-tooltip-autocomplete": {
      fontSize: "10.5px",
    },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
      backgroundColor: "#142334",
      color: "#e8f0f8",
    },
    ".cm-tooltip-autocomplete > ul > li": {
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      lineHeight: "1.5",
    },
    ".cm-panels": {
      backgroundColor: "#0d141d",
      color: "#cdd8e4",
      borderColor: "#263140",
    },
    ".cm-panels.cm-panels-top": {
      borderBottom: "1px solid #263140",
    },
    ".cm-panel.cm-search": {
      padding: "6px",
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: "10.5px",
    },
    ".cm-panel.cm-search input, .cm-panel.cm-search button": {
      backgroundColor: "#101824",
      color: "#cdd8e4",
      border: "1px solid #2b3542",
      borderRadius: "3px",
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: "10.5px",
    },
    ".cm-panel.cm-search label": {
      color: "var(--muted)",
      fontSize: "10px",
    },
    ".cm-searchMatch": {
      backgroundColor: "rgba(230, 180, 85, 0.28)",
      outline: "1px solid rgba(230, 180, 85, 0.65)",
    },
    ".cm-searchMatch.cm-searchMatch-selected": {
      backgroundColor: "rgba(230, 180, 85, 0.5)",
    },
    ".cm-breakpoint-gutter": {
      width: "14px",
      cursor: "pointer",
    },
    ".cm-bp": {
      display: "block",
      width: "8px",
      height: "8px",
      margin: "4px auto 0",
      borderRadius: "50%",
      backgroundColor: "#ef6b6b",
    },
    ".cm-bp-spacer": {
      display: "block",
      width: "8px",
      height: "8px",
      margin: "4px auto 0",
    },
    ".cm-debug-line": {
      backgroundColor: "rgba(232, 195, 90, 0.12)",
      borderBottom: "1px solid rgba(232, 195, 90, 0.45)",
    },
    ".cm-issue-line": {
      backgroundColor: "rgba(194, 56, 56, 0.08)",
      borderBottom: "1px solid rgba(194, 56, 56, 0.45)",
    },
    ".cm-foldPlaceholder": {
      backgroundColor: "#142334",
      border: "1px solid #2d4055",
      color: "#8fa0b3",
    },
  },
  { dark: true },
);

export function createScriptEditor(
  parent: HTMLElement,
  doc: string,
  callbacks: ScriptEditorCallbacks,
): ScriptEditorApi {
  let syncing = false;
  let view: EditorView;
  let api: ScriptEditorApi;

  const onUpdate = (update: { docChanged: boolean; selectionSet: boolean; state: EditorState }) => {
    if (update.docChanged && !syncing) {
      callbacks.onChange(update.state.doc.toString());
    }
    if (update.selectionSet || update.docChanged) {
      const position = update.state.selection.main.head;
      const coords = view.coordsAtPos(position);
      const rect = parent.getBoundingClientRect();
      callbacks.onCursor?.(
        update.state.doc.toString(),
        position,
        coords ? { left: coords.left - rect.left, top: coords.bottom - rect.top } : null,
      );
    }
  };

  const completionSource = (context: {
    state: EditorState;
    pos: number;
    explicit: boolean;
  }) => {
    const result = callbacks.completion(
      context.state.doc.toString(),
      context.pos,
      context.explicit,
    );
    if (!result) return null;
    return {
      from: result.start,
      to: result.end,
      filter: false,
      options: result.items.map((item) => ({
        label: item.label,
        detail: item.detail,
        apply: (target: EditorView, _completion: unknown, from: number, to: number) => {
          target.dispatch({
            changes: { from, to, insert: item.insert },
            selection: { anchor: from + item.caret },
          });
        },
      })),
    };
  };

  const bindings: readonly KeyBinding[] = [
    {
      key: "F9",
      run: (target) => {
        const line = target.state.doc.lineAt(target.state.selection.main.head).number;
        callbacks.onToggleBreakpoint(line);
        return true;
      },
    },
    { key: "Alt-/", run: startCompletion },
    indentWithTab,
  ];

  const breakpointGutter = gutter({
    class: "cm-breakpoint-gutter",
    markers: (target) => target.state.field(breakpointsField),
    initialSpacer: () => new BreakpointSpacer(),
    domEventHandlers: {
      mousedown: (view, line) => {
        callbacks.onToggleBreakpoint(view.state.doc.lineAt(line.from).number);
        return true;
      },
    },
  });

  const toggleBreakpointAtLine = (target: EditorView, line: { from: number }) => {
    callbacks.onToggleBreakpoint(target.state.doc.lineAt(line.from).number);
    return true;
  };

  const extensions = [
    breakpointGutter,
    lineNumbers({
      domEventHandlers: { mousedown: toggleBreakpointAtLine },
    }),
    highlightActiveLineGutter(),
    highlightSpecialChars(),
    history(),
    foldGutter(),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    scriptLanguage,
    scriptIndentUnit,
    scriptIndent,
    scriptFold,
    syntaxHighlighting(scriptHighlightStyle),
    bracketMatching(),
    closeBrackets(),
    autocompletion({ override: [completionSource] }),
    highlightActiveLine(),
    highlightSelectionMatches(),
    search({ top: true }),
    breakpointsField,
    debugField,
    issueField,
    EditorView.decorations.from(debugField),
    EditorView.decorations.from(issueField),
    EditorView.updateListener.of(onUpdate),
    keymap.of([
      ...bindings,
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      ...foldKeymap,
      ...completionKeymap,
    ]),
    placeholder("on start { … }"),
    editorTheme,
  ];

  const makeState = (text: string) =>
    EditorState.create({ doc: text, extensions });

  view = new EditorView({
    state: makeState(doc),
    parent,
  });

  api = {
    setDoc(next, reset) {
      if (reset) {
        const selection = view.state.selection;
        view.setState(makeState(next));
        if (selection.main.head <= next.length) {
          view.dispatch({ selection });
        }
        return;
      }
      if (view.state.doc.toString() === next) return;
      syncing = true;
      try {
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: next },
        });
      } finally {
        syncing = false;
      }
    },
    setBreakpoints(lines) {
      view.dispatch({ effects: setBreakpoints.of(lines) });
    },
    setDebugLine(line) {
      view.dispatch({ effects: setDebugLine.of(line) });
    },
    setIssueLine(line) {
      view.dispatch({ effects: setIssueLine.of(line) });
    },
    revealLine(line, column = 1) {
      if (line < 1 || line > view.state.doc.lines) return;
      const target = view.state.doc.line(line);
      const from = Math.min(target.to, target.from + Math.max(0, column - 1));
      view.dispatch({ selection: { anchor: from }, scrollIntoView: true });
      view.focus();
    },
    focus() {
      view.focus();
    },
    destroy() {
      view.destroy();
    },
  };
  return api;
}
