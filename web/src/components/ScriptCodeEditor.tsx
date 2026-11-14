import { useEffect, useLayoutEffect, useRef } from "preact/hooks";
import {
  createScriptEditor,
  type ScriptCompletionResult,
  type ScriptEditorApi,
} from "../scriptEditor";

export interface ScriptCodeEditorProps {
  doc: string;
  scriptId: number | null;
  breakpoints: number[];
  debugLine: number | null;
  issueLine: number | null;
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
  apiRef: { current: ScriptEditorApi | null };
}

export function ScriptCodeEditor({
  doc,
  scriptId,
  breakpoints,
  debugLine,
  issueLine,
  completion,
  onChange,
  onToggleBreakpoint,
  onCursor,
  apiRef,
}: ScriptCodeEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const callbacks = useRef({ completion, onChange, onToggleBreakpoint, onCursor });
  callbacks.current = { completion, onChange, onToggleBreakpoint, onCursor };
  const latest = useRef({ breakpoints, debugLine, issueLine });
  latest.current = { breakpoints, debugLine, issueLine };
  const previousScript = useRef<number | null>(scriptId);

  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const api = createScriptEditor(host, doc, {
      completion: (text, position, explicit) =>
        callbacks.current.completion(text, position, explicit),
      onChange: (text) => callbacks.current.onChange(text),
      onToggleBreakpoint: (line) => callbacks.current.onToggleBreakpoint(line),
      onCursor: (text, position, coords) =>
        callbacks.current.onCursor?.(text, position, coords),
    });
    apiRef.current = api;
    api.setBreakpoints(latest.current.breakpoints);
    api.setDebugLine(latest.current.debugLine);
    api.setIssueLine(latest.current.issueLine);
    return () => {
      if (apiRef.current === api) apiRef.current = null;
      api.destroy();
    };
    // The editor instance lives for the modal's lifetime; props flow through
    // the effects below, so the initial doc is the only one this needs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const api = apiRef.current;
    if (!api) return;
    const reset = previousScript.current !== scriptId;
    previousScript.current = scriptId;
    api.setDoc(doc, reset);
    if (reset) {
      api.setBreakpoints(latest.current.breakpoints);
      api.setDebugLine(latest.current.debugLine);
      api.setIssueLine(latest.current.issueLine);
    }
  }, [apiRef, doc, scriptId]);

  useEffect(() => {
    apiRef.current?.setBreakpoints(breakpoints);
  }, [apiRef, breakpoints]);

  useEffect(() => {
    apiRef.current?.setDebugLine(debugLine);
  }, [apiRef, debugLine]);

  useEffect(() => {
    apiRef.current?.setIssueLine(issueLine);
  }, [apiRef, issueLine]);

  return <div class="script-cm" ref={hostRef} />;
}
