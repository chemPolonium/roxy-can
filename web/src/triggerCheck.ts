import type {
  Bus,
  Catalog,
  DerivedDef,
  FrTxEntry,
  ScriptInfo,
  SysVar,
  TriggerEntry,
  TxEntry,
} from "./types";

export interface TriggerCheckContext {
  buses: Bus[];
  sysvars: SysVar[];
  derived: DerivedDef[];
  tx: TxEntry[];
  frTx: FrTxEntry[];
  scripts: ScriptInfo[];
  catalog: Catalog | null;
}

const GLOBAL_KINDS = new Set(["sysvar", "derived"]);
const BUS_KINDS = new Set(["fr_frame", "fr_signal", "fr_timeout", "eth_flow"]);

/// Static reference check: every target a rule names should exist in this
/// project, so "armed but can never fire" shows up before the test run.
/// It does not replay traffic; the Write log still reports runtime misses.
export function checkTriggers(
  triggers: TriggerEntry[],
  ctx: TriggerCheckContext,
): string[] {
  const issues: string[] = [];
  const busByIndex = new Map(ctx.buses.map((bus) => [bus.index, bus]));
  const sysvarKeys = new Set(
    ctx.sysvars.map((entry) => `${entry.namespace}::${entry.name}`),
  );
  const derivedIds = new Set(ctx.derived.map((entry) => entry.id));
  const scriptActions = new Set(
    ctx.scripts.flatMap((script) => script.panelActions),
  );
  for (let index = 0; index < triggers.length; index += 1) {
    const rule = triggers[index];
    const label = `Rule ${index + 1}`;
    if (!GLOBAL_KINDS.has(rule.kind)) {
      const target = BUS_KINDS.has(rule.kind) ? rule.bus : rule.ch;
      if (!busByIndex.has(target)) {
        issues.push(`${label}: bus ${target} is not in this project`);
      }
    }
    if (rule.kind === "sysvar" && !sysvarKeys.has(rule.varKey)) {
      issues.push(`${label}: sysvar "${rule.varKey}" is not declared`);
    }
    if (rule.kind === "derived" && !derivedIds.has(rule.derivedId)) {
      issues.push(`${label}: derived channel "${rule.derivedId}" is missing`);
    }
    if (rule.kind === "signal" && ctx.catalog) {
      const channel = ctx.catalog.channels.find(
        (entry) => entry.kind === "can" && entry.index === rule.ch,
      );
      const message = channel?.messages.find(
        (entry) => entry.id === rule.id && entry.extended === rule.ext,
      );
      if (!message) {
        issues.push(`${label}: CAN message ${rule.id.toString(16)} is not in the catalog`);
      } else if (!message.signals.some((signal) => signal.name === rule.signal)) {
        issues.push(`${label}: signal "${rule.signal}" is not in that message`);
      }
    }
    if (rule.actionKind === "send") {
      if (!ctx.tx.some((entry) => entry.channel === rule.sendCh && entry.id === rule.sendId)) {
        issues.push(`${label}: no CAN generator entry for the send target`);
      }
    } else if (rule.actionKind === "send_fr") {
      if (!ctx.frTx.some((entry) => entry.bus === rule.sendCh && entry.slot === rule.sendId)) {
        issues.push(`${label}: no FlexRay generator entry for the send target`);
      }
    } else if (rule.actionKind === "send_eth") {
      const bus = busByIndex.get(rule.sendCh);
      if (!bus || bus.kind !== "ethernet") {
        issues.push(`${label}: Ethernet send bus ${rule.sendCh} is missing`);
      } else if (!rule.sendFrame.trim()) {
        issues.push(`${label}: no Ethernet frame name for the send target`);
      }
    } else if (rule.actionKind === "set_sysvar" && !sysvarKeys.has(rule.sysVarKey)) {
      issues.push(`${label}: sysvar "${rule.sysVarKey}" is not declared`);
    } else if (rule.actionKind === "script" && !scriptActions.has(rule.scriptName)) {
      issues.push(`${label}: no script declares panel action "${rule.scriptName}"`);
    }
  }
  return issues;
}
