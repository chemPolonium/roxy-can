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
  it("offers only handler events after on", () => {
    const items = labels("on |");
    expect(items).toEqual([
      "start",
      "stop",
      "message",
      "extended",
      "timer",
      "errorFrame",
      "fr",
    ]);
    expect(items).not.toContain("print");
  });

  it("narrows the event word by prefix", () => {
    expect(labels("on me|")).toEqual(["message"]);
    expect(labels("on extended |").map((label) => label)).toEqual(["message"]);
  });

  it("lists DBC messages after on message, including the wildcard", () => {
    const items = labels("on message |");
    expect(items).toContain("EngineStatus");
    expect(items).toContain("BrakeInfo");
    expect(items).toContain("*");
    expect(items).not.toContain("print");
  });

  it("lists a message's signals after its dot", () => {
    expect(labels("EngineStatus.|")).toEqual(["EngineSpeed"]);
  });

  it("completes message names inside expressions", () => {
    expect(labels("fn helper() {\n    Eng|\n}")).toContain("EngineStatus");
    expect(labels("on start {\n    Bra|\n}")).toContain("BrakeInfo");
  });

  it("lists message names inside quoted handler targets", () => {
    expect(labels('on message "En|')).toContain("EngineStatus");
  });
});

describe("local symbols", () => {
  it("keeps outer variables visible in a nested block", () => {
    const items = labels("let alpha = 1;\non start {\n    al|\n}");
    expect(items).toContain("alpha");
  });

  it("hides variables from a closed sibling block", () => {
    expect(labels("if (true) { let hidden = 1; }\n    hid|")).not.toContain(
      "hidden",
    );
  });

  it("offers the current function's parameters", () => {
    const result = complete("fn helper(alpha) {\n    al|\n}");
    const alpha = result?.items.find((item) => item.label === "alpha");
    expect(alpha?.detail).toBe("local · parameter");
  });

  it("offers top-level functions declared later", () => {
    const result = complete("hel|\nfn helper() {}");
    const helper = result?.items.find((item) => item.label === "helper");
    expect(helper?.detail).toBe("script function");
    expect(helper?.insert).toBe("helper()");
  });

  it("keeps local function parameters for signature help", () => {
    const symbols = localSymbols("fn helper(alpha, beta) {\n}", 0);
    expect(symbols.find((symbol) => symbol.name === "helper")?.params).toEqual([
      "alpha",
      "beta",
    ]);
  });

  it("lets a local shadow a builtin without inserting a call", () => {
    const result = complete("let print = 1;\npri|");
    const prints = result?.items.filter((item) => item.label === "print");
    expect(prints).toHaveLength(1);
    expect(prints?.[0].detail).toBe("local · let");
    expect(prints?.[0].insert).toBe("print");
  });

  it("ignores declarations in comments and strings", () => {
    const source = '// let commented = 1;\nlet real = 2;\n"let quoted = 3;"';
    expect(localSymbols(source, source.length)).toEqual([
      { name: "real", kind: "let" },
    ]);
  });
});

describe("highlight lines", () => {
  it("marks functions, strings and comments on one line", () => {
    const line = highlightLine('    print("x") // note', false, false);
    expect(line.block).toBe(false);
    expect(line.html).toContain('class="tok-fn">print</span>');
    expect(line.html).toContain('class="tok-string"');
    expect(line.html).toContain('class="tok-comment"');
  });

  it("marks keywords and numbers", () => {
    const line = highlightLine("let value = 42;", false, false);
    expect(line.html).toContain('class="tok-keyword">let</span>');
    expect(line.html).toContain('class="tok-number">42</span>');
  });

  it("carries a block comment across lines", () => {
    const first = highlightLine("/* block", false, false);
    expect(first.block).toBe(true);
    expect(first.html).toContain('class="tok-comment"');
    const second = highlightLine("   end */ print(1);", true, false);
    expect(second.block).toBe(false);
    expect(second.html).toContain('class="tok-comment"');
    expect(second.html).toContain('class="tok-fn">print</span>');
    expect(second.html).toContain('class="tok-number">1</span>');
  });

  it("opens a block comment after code", () => {
    const line = highlightLine("let x = 1; /* open", false, false);
    expect(line.block).toBe(true);
    expect(line.html).toContain('class="tok-number">1</span>');
    expect(line.html).toContain('class="tok-comment">/* open</span>');
  });
});

describe("find and signature helpers", () => {
  it("finds case-insensitive matches", () => {
    expect(findMatches("Alpha alpha ALPHA", "alpha")).toEqual([0, 6, 12]);
  });

  it("finds the innermost call and its active argument", () => {
    expect(callAtCaret("print()", 6)).toEqual({ name: "print", active: 0 });
    expect(callAtCaret("get_sig(buffer, )", 16)).toEqual({
      name: "get_sig",
      active: 1,
    });
    expect(
      callAtCaret("print(\n    1,\n    )", "print(\n    1,\n    ".length),
    ).toEqual({ name: "print", active: 1 });
    expect(callAtCaret("if ()", 4)).toEqual({ name: "if", active: 0 });
  });

  it("splits usage strings without breaking quoted commas", () => {
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
