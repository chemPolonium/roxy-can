import { describe, expect, it } from "vitest";
import type {
  Catalog,
  DerivedDef,
  ScriptFunctionHelp,
  SysVar,
} from "../types";
import {
  buildCandidates,
  callAtCaret,
  findMatches,
  highlightLine,
  localSymbols,
  signatureArgs,
} from "./ScriptsModal";

const catalog: Catalog = {
  channels: [
    {
      kind: "can",
      index: 0,
      name: "CAN1",
      messages: [
        {
          id: 0x100,
          extended: false,
          name: "EngineStatus",
          signals: [
            {
              key: "0:256:EngineSpeed",
              name: "EngineSpeed",
              unit: "rpm",
              min: 0,
              max: 8000,
            },
          ],
        },
        {
          id: 0x320,
          extended: false,
          name: "BrakeInfo",
          signals: [],
        },
        {
          id: 0x321,
          extended: false,
          name: "Frame Name",
          signals: [
            {
              key: "0:801:Value",
              name: "Value",
              unit: "",
              min: 0,
              max: 100,
            },
          ],
        },
      ],
    },
  ],
};

const help: ScriptFunctionHelp[] = [
  { name: "print", usage: "print(value, …)" },
  { name: "get_sig", usage: 'get_sig(buffer, id, "Name")' },
  { name: "now", usage: "now()" },
];

const derived: DerivedDef[] = [];
const sysvars: SysVar[] = [];

function complete(marked: string, allowEmpty = false) {
  const caret = marked.indexOf("|");
  const value = marked.replace("|", "");
  return buildCandidates(
    value,
    caret,
    allowEmpty,
    help,
    derived,
    sysvars,
    catalog,
    0,
  );
}

function labels(marked: string, allowEmpty = false): string[] {
  return complete(marked, allowEmpty)?.items.map((item) => item.label) ?? [];
}

describe("completion contexts", () => {
  it("offers catalog frame paths inside frame(...)", () => {
    const paths = labels("on start { let f = frame(|", true);
    expect(paths).toContain("CAN1.EngineStatus");
    expect(paths).toContain("EngineStatus");
    expect(paths.every((path) => path !== "0:256:EngineSpeed")).toBe(true);
    const narrowed = labels("on start { let f = frame(CAN1.|", true);
    expect(narrowed).toEqual(
      expect.arrayContaining(["CAN1.EngineStatus", "CAN1.BrakeInfo"]),
    );
  });

  it("offers a frame instance's signals after its dot", () => {
    const fields = labels(
      "on start {\n    let f = frame(CAN1.EngineStatus);\n    f.|",
      true,
    );
    expect(fields).toContain("EngineSpeed");
    expect(fields).not.toContain("EngineTemp");
  });

  it("completes quoted frame names and bracket paths", () => {
    expect(labels('on start { let f = frame(CAN1["Fra|', true)).toEqual([
      "Frame Name",
    ]);
    expect(labels('on start { let f = frame("Fra|', true)).toEqual([
      "Frame Name",
    ]);
    const inserts =
      complete("on start { let f = frame(|", true)?.items.map((i) => i.insert) ??
      [];
    expect(inserts).toContain('CAN1["Frame Name"]');
    expect(inserts).toContain('"Frame Name"');
    const fields = labels(
      'on start {\n    let f = frame(CAN1["Frame Name"]);\n    f.|',
      true,
    );
    expect(fields).toEqual(["Value"]);
  });

  it("follows a re-assigned or argument-held frame instance", () => {
    const reassigned = labels(
      "on start {\n    let f = frame(CAN1.BrakeInfo);\n    f = frame(CAN1.EngineStatus);\n    f.|",
      true,
    );
    expect(reassigned).toContain("EngineSpeed");
    const argument = labels(
      "fn send(cmd) {\n    cmd.|\n}\non start { send(frame(CAN1.EngineStatus)); }",
      true,
    );
    expect(argument).toEqual(["EngineSpeed"]);
  });

  it("offers only handler events after on", () => {
    const items = labels("on |");
    expect(items).toEqual([
      "start",
      "stop",
      "panel_action",
      "message",
      "extended",
      "timer",
      "errorFrame",
      "fr",
    ]);
    expect(items).not.toContain("print");
  });

  it("narrows and lists event targets", () => {
    expect(labels("on me|")).toEqual(["message"]);
    expect(labels("on extended |")).toEqual(["message"]);
    const messages = labels("on message |");
    expect(messages).toContain("EngineStatus");
    expect(messages).toContain("BrakeInfo");
    expect(messages).toContain("*");
    expect(messages).not.toContain("print");
    expect(labels("EngineStatus.|")).toEqual(["EngineSpeed"]);
  });

  it("completes message names in expressions and quoted targets", () => {
    expect(labels("fn helper() {\n    Eng|\n}")).toContain("EngineStatus");
    expect(labels("on start {\n    Bra|\n}")).toContain("BrakeInfo");
    expect(labels('on message "En|')).toContain("EngineStatus");
  });
});

describe("local symbols", () => {
  it("scopes locals, functions and parameters", () => {
    expect(labels("let alpha = 1;\non start {\n    al|\n}")).toContain(
      "alpha",
    );
    expect(labels("if (true) { let hidden = 1; }\n    hid|")).not.toContain(
      "hidden",
    );
    const param = complete("fn helper(alpha) {\n    al|\n}");
    expect(
      param?.items.find((item) => item.label === "alpha")?.detail,
    ).toBe("local · parameter");
    const later = complete("hel|\nfn helper() {}");
    const helper = later?.items.find((item) => item.label === "helper");
    expect(helper?.detail).toBe("script function");
    expect(helper?.insert).toBe("helper()");
    const symbols = localSymbols("fn helper(alpha, beta) {\n}", 0);
    expect(symbols.find((symbol) => symbol.name === "helper")?.params).toEqual([
      "alpha",
      "beta",
    ]);
    const shadowed = complete("let print = 1;\npri|");
    const prints = shadowed?.items.filter((item) => item.label === "print");
    expect(prints).toHaveLength(1);
    expect(prints?.[0].detail).toBe("local · let");
    expect(prints?.[0].insert).toBe("print");
    const source = '// let commented = 1;\nlet real = 2;\n"let quoted = 3;"';
    expect(localSymbols(source, source.length)).toEqual([
      { name: "real", kind: "let" },
    ]);
  });
});

describe("highlight lines", () => {
  it("marks tokens and carries block comments across lines", () => {
    const line = highlightLine('    print("x") // note', false, false);
    expect(line.block).toBe(false);
    expect(line.html).toContain('class="tok-fn">print</span>');
    expect(line.html).toContain('class="tok-string"');
    expect(line.html).toContain('class="tok-comment"');
    const value = highlightLine("let value = 42;", false, false);
    expect(value.html).toContain('class="tok-keyword">let</span>');
    expect(value.html).toContain('class="tok-number">42</span>');
    const first = highlightLine("/* block", false, false);
    expect(first.block).toBe(true);
    expect(first.html).toContain('class="tok-comment"');
    const second = highlightLine("   end */ print(1);", true, false);
    expect(second.block).toBe(false);
    expect(second.html).toContain('class="tok-comment"');
    expect(second.html).toContain('class="tok-fn">print</span>');
    expect(second.html).toContain('class="tok-number">1</span>');
    const open = highlightLine("let x = 1; /* open", false, false);
    expect(open.block).toBe(true);
    expect(open.html).toContain('class="tok-number">1</span>');
    expect(open.html).toContain('class="tok-comment">/* open</span>');
  });
});

describe("find and signature helpers", () => {
  it("finds matches, calls and usage arguments", () => {
    expect(findMatches("Alpha alpha ALPHA", "alpha")).toEqual([0, 6, 12]);
    const calls: [string, number, { name: string; active: number }][] = [
      ["print()", 6, { name: "print", active: 0 }],
      ["get_sig(buffer, )", 16, { name: "get_sig", active: 1 }],
      [
        "print(\n    1,\n    )",
        "print(\n    1,\n    ".length,
        { name: "print", active: 1 },
      ],
      ["if ()", 4, { name: "if", active: 0 }],
    ];
    for (const [source, caret, expected] of calls) {
      expect(callAtCaret(source, caret), source).toEqual(expected);
    }
    expect(signatureArgs('get_sig(buffer, id, "Name")')).toEqual([
      "buffer",
      "id",
      '"Name"',
    ]);
    expect(signatureArgs("now()")).toEqual([]);
    expect(signatureArgs('emit_value("Name", value)')).toEqual([
      '"Name"',
      "value",
    ]);
  });
});
