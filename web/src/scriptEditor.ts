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
  Compartment,
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
  type EditorTheme,
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
  onApply?: () => void;
  onCursor?: (
    doc: string,
    position: number,
    coords: { left: number; top: number } | null,
  ) => void;
}

export interface ScriptEditorApi {
  setDoc: (doc: string, reset: boolean) => void;
  setTheme: (theme: EditorTheme) => void;
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

function editorTheme(theme: EditorTheme) {
  const dark = theme === "dark";
  const c = dark
    ? {
        text: "#d6e2f0",
        gutterBg: "#0d141d",
        gutterBorder: "#263140",
        gutterText: "#5d6a7a",
        active: "rgba(76, 157, 255, 0.06)",
        activeGutter: "#111a25",
        caret: "#e8f0f8",
        selection: "rgba(76, 157, 255, 0.35)",
        tooltipBg: "#0d141d",
        tooltipBorder: "#2d4055",
        tooltipText: "#cdd8e4",
        selected: "#142334",
        selectedText: "#e8f0f8",
        panelBg: "#0d141d",
        panelText: "#cdd8e4",
        panelBorder: "#263140",
        inputBg: "#101824",
        search: "rgba(230, 180, 85, 0.28)",
        searchOutline: "rgba(230, 180, 85, 0.65)",
        searchSelected: "rgba(230, 180, 85, 0.5)",
        foldBg: "#142334",
        foldBorder: "#2d4055",
        foldText: "#8fa0b3",
        debugBg: "rgba(232, 195, 90, 0.12)",
        debugBorder: "rgba(232, 195, 90, 0.45)",
        issueBg: "rgba(194, 56, 56, 0.08)",
        issueBorder: "rgba(194, 56, 56, 0.45)",
        bp: "#ef6b6b",
      }
    : {
        text: "#26313f",
        gutterBg: "#eef2f7",
        gutterBorder: "#d4dbe4",
        gutterText: "#8a95a5",
        active: "rgba(31, 111, 208, 0.06)",
        activeGutter: "#e6edf6",
        caret: "#1f2937",
        selection: "rgba(31, 111, 208, 0.18)",
        tooltipBg: "#ffffff",
        tooltipBorder: "#cdd8e4",
        tooltipText: "#243244",
        selected: "#e8f1fd",
        selectedText: "#1f2937",
        panelBg: "#f7f9fb",
        panelText: "#243244",
        panelBorder: "#d5dce5",
        inputBg: "#ffffff",
        search: "rgba(161, 95, 0, 0.2)",
        searchOutline: "rgba(161, 95, 0, 0.45)",
        searchSelected: "rgba(161, 95, 0, 0.32)",
        foldBg: "#eef2f7",
        foldBorder: "#cdd8e4",
        foldText: "#526174",
        debugBg: "rgba(154, 107, 0, 0.10)",
        debugBorder: "rgba(154, 107, 0, 0.40)",
        issueBg: "rgba(194, 56, 56, 0.08)",
        issueBorder: "rgba(194, 56, 56, 0.45)",
        bp: "#d64545",
      };
  return EditorView.theme(
    {
      "&": {
        height: "100%",
        backgroundColor: "transparent",
        color: c.text,
      },
      ".cm-scroller": {
        fontFamily:
          '"Roxy Code", "Cascadia Mono", Consolas, "Microsoft YaHei", monospace',
        fontSize: "12px",
        lineHeight: "1.5",
      },
      ".cm-content": {
        padding: "8px 0",
        caretColor: c.caret,
      },
      ".cm-line": {
        padding: "0 8px",
      },
      ".cm-gutters": {
        backgroundColor: c.gutterBg,
        borderRight: `1px solid ${c.gutterBorder}`,
        color: c.gutterText,
      },
      ".cm-activeLine": {
        backgroundColor: c.active,
      },
      ".cm-activeLineGutter": {
        backgroundColor: c.activeGutter,
      },
      ".cm-cursor": {
        borderLeftColor: c.caret,
      },
      ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection":
        {
          backgroundColor: c.selection,
        },
      ".cm-tooltip": {
        backgroundColor: c.tooltipBg,
        border: `1px solid ${c.tooltipBorder}`,
        color: c.tooltipText,
      },
      ".cm-tooltip-autocomplete": {
        fontSize: "10.5px",
      },
      ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
        backgroundColor: c.selected,
        color: c.selectedText,
      },
      ".cm-tooltip-autocomplete > ul > li": {
        fontFamily: '"Roxy Code", "Cascadia Mono", Consolas, monospace',
        lineHeight: "1.5",
      },
      ".cm-panels": {
        backgroundColor: c.panelBg,
        color: c.panelText,
        borderColor: c.panelBorder,
      },
      ".cm-panels.cm-panels-top": {
        borderBottom: `1px solid ${c.panelBorder}`,
      },
      ".cm-panel.cm-search": {
        padding: "6px",
        fontFamily: '"Roxy Code", "Cascadia Mono", Consolas, monospace',
        fontSize: "10.5px",
      },
      ".cm-panel.cm-search input, .cm-panel.cm-search button": {
        backgroundColor: c.inputBg,
        color: c.panelText,
        border: `1px solid ${c.panelBorder}`,
        borderRadius: "3px",
        fontFamily: '"Roxy Code", "Cascadia Mono", Consolas, monospace',
        fontSize: "10.5px",
      },
      ".cm-panel.cm-search label": {
        color: c.gutterText,
        fontSize: "10px",
      },
      ".cm-searchMatch": {
        backgroundColor: c.search,
        outline: `1px solid ${c.searchOutline}`,
      },
      ".cm-searchMatch.cm-searchMatch-selected": {
        backgroundColor: c.searchSelected,
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
        backgroundColor: c.bp,
      },
      ".cm-bp-spacer": {
        display: "block",
        width: "8px",
        height: "8px",
        margin: "4px auto 0",
      },
      ".cm-debug-line": {
        backgroundColor: c.debugBg,
        borderBottom: `1px solid ${c.debugBorder}`,
      },
      ".cm-issue-line": {
        backgroundColor: c.issueBg,
        borderBottom: `1px solid ${c.issueBorder}`,
      },
      ".cm-foldPlaceholder": {
        backgroundColor: c.foldBg,
        border: `1px solid ${c.foldBorder}`,
        color: c.foldText,
      },
    },
    { dark },
  );
}
export function createScriptEditor(
  parent: HTMLElement,
  doc: string,
  theme: EditorTheme,
  callbacks: ScriptEditorCallbacks,
): ScriptEditorApi {
  let syncing = false;
  let view: EditorView;
  let api: ScriptEditorApi;
  const themeCompartment = new Compartment();
  const highlightCompartment = new Compartment();

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
      key: "Mod-s",
      run: () => {
        callbacks.onApply?.();
        return true;
      },
    },
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
    highlightCompartment.of(syntaxHighlighting(scriptHighlightStyle(theme))),
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
    themeCompartment.of(editorTheme(theme)),
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
    setTheme(next) {
      view.dispatch({
        effects: [
          themeCompartment.reconfigure(editorTheme(next)),
          highlightCompartment.reconfigure(
            syntaxHighlighting(scriptHighlightStyle(next)),
          ),
        ],
      });
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
