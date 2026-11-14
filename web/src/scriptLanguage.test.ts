import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { foldable, getIndentation } from "@codemirror/language";
import {
  scriptFold,
  scriptIndent,
  scriptIndentUnit,
  scriptLanguage,
} from "./scriptLanguage";

describe("rxcan language support", () => {
  it("indents a braced block and outdents its closing brace", () => {
    const state = EditorState.create({
      doc: "on start {\n\n}\n",
      extensions: [scriptLanguage, scriptIndentUnit, scriptIndent],
    });
    expect(getIndentation(state, state.doc.line(2).from)).toBe(4);
    expect(getIndentation(state, state.doc.line(3).from)).toBe(0);
  });

  it("folds a line that ends in an opening brace", () => {
    const state = EditorState.create({
      doc: "on start {\n    print(1);\n}\n",
      extensions: [scriptLanguage, scriptFold],
    });
    const first = state.doc.line(1);
    const close = state.doc.line(3);
    expect(foldable(state, first.from, first.to)).toEqual({
      from: first.to,
      to: close.from,
    });
  });
});
