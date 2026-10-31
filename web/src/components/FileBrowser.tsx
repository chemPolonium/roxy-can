import { createContext } from "preact";
import type { ComponentChildren } from "preact";
import {
  useCallback,
  useContext,
  useEffect,
  useState,
} from "preact/hooks";
import { listDirectory, createFolder } from "../api";
import type { FsEntry, FsList } from "../types";

export interface FilePickOptions {
  mode: "file" | "folder" | "save";
  title?: string;
  accept?: string[];
  initialPath?: string | null;
  initialName?: string;
}

interface FilePickRequest extends FilePickOptions {
  resolve: (path: string | null) => void;
}

type PickFn = (options: FilePickOptions) => Promise<string | null>;

const FileBrowserContext = createContext<PickFn>(() => Promise.resolve(null));

export function useFileBrowser(): PickFn {
  return useContext(FileBrowserContext);
}

/// Underlying dialogs also close on Escape; while the host browser is open
/// they must leave that key to it.
export function fileBrowserOpen(): boolean {
  return Boolean(document.querySelector(".file-browser-modal"));
}

function parseAccept(accept?: string): string[] {
  return (accept ?? "")
    .split(",")
    .map((item) => item.trim().toLowerCase().replace(/^\./, ""))
    .filter((item) => /^[a-z0-9]+$/.test(item));
}

export function HostPathPick({
  label,
  accept,
  mode = "file",
  initialPath,
  onPick,
  onError,
  class: cls = "mini",
  title,
}: {
  label: string;
  accept?: string;
  mode?: "file" | "folder";
  initialPath?: string | null;
  onPick: (path: string) => void | Promise<void>;
  onError?: (message: string) => void;
  class?: string;
  title?: string;
}) {
  const pick = useFileBrowser();
  return (
    <button
      type="button"
      class={cls}
      title={title}
      onClick={async () => {
        try {
          const path = await pick({
            mode,
            accept: mode === "file" ? parseAccept(accept) : [],
            initialPath,
            title: mode === "folder" ? "Choose folder" : "Choose file",
          });
          if (path) await onPick(path);
        } catch (reason) {
          onError?.(reason instanceof Error ? reason.message : String(reason));
        }
      }}
    >
      {label}
    </button>
  );
}

export function FileBrowserProvider({ children }: { children: ComponentChildren }) {
  const [request, setRequest] = useState<FilePickRequest | null>(null);
  const [lastFolder, setLastFolder] = useState<string | null>(null);
  const pick = useCallback(
    (options: FilePickOptions) => {
      return new Promise<string | null>((resolve) => {
        setRequest((current) => {
          current?.resolve(null);
          return {
            ...options,
            initialPath: options.initialPath ?? lastFolder,
            resolve,
          };
        });
      });
    },
    [lastFolder],
  );
  const close = (path: string | null) => {
    const current = request;
    if (path) {
      const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
      setLastFolder(
        current?.mode === "folder" || cut <= 0 ? path : path.slice(0, cut),
      );
    }
    setRequest(null);
    current?.resolve(path);
  };
  return (
    <FileBrowserContext.Provider value={pick}>
      {children}
      {request && (
        <FileBrowser
          mode={request.mode}
          title={request.title}
          accept={request.accept}
          initialPath={request.initialPath}
          initialName={request.initialName}
          onClose={close}
        />
      )}
    </FileBrowserContext.Provider>
  );
}

function extension(name: string): string {
  const cut = name.lastIndexOf(".");
  return cut >= 0 ? name.slice(cut + 1).toLowerCase() : "";
}

function ensureExtension(name: string, accept: string[]): string {
  if (accept.length !== 1) return name;
  const wanted = accept[0].toLowerCase();
  if (extension(name) === wanted) return name;
  const stem = name.replace(/\.[^./\\]+$/, "");
  return `${stem}.${wanted}`;
}

function joinPath(folder: string, name: string): string {
  if (folder.includes("\\")) {
    return `${folder.replace(/[\\/]+$/, "")}\\${name}`;
  }
  return `${folder.replace(/[\\/]+$/, "")}/${name}`;
}

function EntryIcon({ dir }: { dir: boolean }) {
  return (
    <svg class="fb-icon" viewBox="0 0 16 16" aria-hidden="true">
      {dir ? (
        <path d="M1.5 4.5h4l1.5 2h7v6.5h-12.5z" />
      ) : (
        <>
          <path d="M3.5 1.5h5l3 3v10h-8z" />
          <path d="M8.5 1.5v3h3" />
        </>
      )}
    </svg>
  );
}

type SortKey = "name" | "modified" | "size";

function FileBrowser({
  mode,
  title,
  accept = [],
  initialPath,
  initialName = "",
  onClose,
}: FilePickOptions & { onClose: (path: string | null) => void }) {
  const [listing, setListing] = useState<FsList | null>(null);
  const [selected, setSelected] = useState<FsEntry | null>(null);
  const [name, setName] = useState(initialName);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  const [newFolder, setNewFolder] = useState("");
  const [createError, setCreateError] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 }>({
    key: "name",
    dir: 1,
  });

  const allowed = (entry: FsEntry) =>
    mode === "folder"
      ? entry.dir
      : entry.dir || accept.length === 0 || accept.includes(extension(entry.name));

  const load = async (path?: string | null) => {
    setLoading(true);
    setError("");
    try {
      const next = await listDirectory(path);
      setListing(next);
      setSelected(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  };

  const sortBy = (key: SortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, dir: current.dir === 1 ? -1 : 1 }
        : { key, dir: key === "name" ? 1 : -1 },
    );
  };

  const create = async () => {
    const folder = newFolder.trim();
    if (!listing || !folder) return;
    setCreateError("");
    try {
      const created = await createFolder(listing.path, folder);
      setCreating(false);
      setNewFolder("");
      await load(created.path);
    } catch (reason) {
      setCreateError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  useEffect(() => {
    void load(initialPath);
    // The dialog is mounted per request; later path changes come from clicks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        // The modal underneath also listens for Escape on window; keep the
        // browser's close from dismissing its owner too. Close on the next
        // turn so that listener still sees the browser in the DOM.
        event.stopImmediatePropagation();
        window.setTimeout(() => onClose(null), 0);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const choose = (path: string | null) => {
    if (path) onClose(path);
  };

  const choosePrimary = () => {
    if (!listing) return;
    if (mode === "folder") {
      choose(listing.path);
    } else if (mode === "save") {
      const stem = name.trim();
      if (stem) choose(joinPath(listing.path, ensureExtension(stem, accept)));
    } else if (selected && !selected.dir) {
      choose(selected.path);
    }
  };

  const entries = (listing?.entries ?? [])
    .filter(allowed)
    .sort((a, b) => {
      if (a.dir !== b.dir) return a.dir ? -1 : 1;
      let cmp = 0;
      if (sort.key === "name") {
        cmp = a.name.localeCompare(b.name, undefined, {
          numeric: true,
          sensitivity: "base",
        });
      } else if (sort.key === "modified") {
        cmp = a.modifiedMs - b.modifiedMs;
      } else {
        cmp = a.size - b.size;
      }
      if (cmp === 0) {
        cmp = a.name.localeCompare(b.name, undefined, {
          numeric: true,
          sensitivity: "base",
        });
      }
      return cmp * sort.dir;
    });
  const primaryDisabled =
    mode === "folder"
      ? !listing
      : mode === "save"
        ? !listing || !name.trim()
        : !selected || selected.dir;

  return (
    <div class="modal-backdrop" onMouseDown={() => onClose(null)}>
      <div
        class="file-browser-modal"
        role="dialog"
        aria-modal="true"
        aria-label={title ?? (mode === "folder" ? "Choose folder" : "Choose file")}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>
            {title ??
              (mode === "folder"
                ? "Choose folder"
                : mode === "save"
                  ? "Save file"
                  : "Choose file")}
          </h2>
          <button type="button" onClick={() => onClose(null)}>
            Close
          </button>
        </header>
        <div class="file-browser-toolbar">
          <button
            type="button"
            class="mini"
            disabled={!listing?.parent || loading}
            title="Go to the parent folder"
            onClick={() => void load(listing?.parent)}
          >
            Up
          </button>
          <select
            class="fb-roots"
            value=""
            title="Quick access"
            onChange={(event) => {
              const path = event.currentTarget.value;
              if (path) void load(path);
            }}
          >
            <option value="">Quick access…</option>
            {(listing?.roots ?? []).map((root) => (
              <option value={root.path} key={root.path}>
                {root.name} — {root.path}
              </option>
            ))}
          </select>
          <input
            class="fb-path"
            value={listing?.path ?? ""}
            readOnly
            aria-label="Current folder"
            spellcheck={false}
          />
          <button
            type="button"
            class="mini"
            disabled={!listing || loading}
            title="Create a folder in this folder"
            onClick={() => {
              setCreating(true);
              setCreateError("");
            }}
          >
            New folder
          </button>
          <button
            type="button"
            class="mini"
            disabled={loading}
            title="Reload this folder"
            onClick={() => void load(listing?.path)}
          >
            Refresh
          </button>
        </div>
        {creating && (
          <form
            class="fb-new"
            onSubmit={(event) => {
              event.preventDefault();
              void create();
            }}
          >
            <input
              autoFocus
              value={newFolder}
              placeholder="Folder name"
              aria-label="New folder name"
              onInput={(event) =>
                setNewFolder((event.target as HTMLInputElement).value)
              }
            />
            <button
              type="submit"
              class="mini"
              disabled={!newFolder.trim() || loading}
            >
              Create
            </button>
            <button
              type="button"
              class="mini"
              onClick={() => {
                setCreating(false);
                setNewFolder("");
                setCreateError("");
              }}
            >
              Cancel
            </button>
            {createError && <span class="fb-error">{createError}</span>}
          </form>
        )}
        <div class="fb-list" role="listbox" aria-label="Folder contents">
          <div class="fb-row fb-head" role="presentation">
            <span />
            <span
              class="fb-sort"
              role="button"
              tabIndex={0}
              onClick={() => sortBy("name")}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  sortBy("name");
                }
              }}
            >
              Name {sort.key === "name" ? (sort.dir === 1 ? "▲" : "▼") : ""}
            </span>
            <span
              class="fb-sort num"
              role="button"
              tabIndex={0}
              onClick={() => sortBy("modified")}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  sortBy("modified");
                }
              }}
            >
              Modified{" "}
              {sort.key === "modified" ? (sort.dir === 1 ? "▲" : "▼") : ""}
            </span>
            <span
              class="fb-sort num"
              role="button"
              tabIndex={0}
              onClick={() => sortBy("size")}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  sortBy("size");
                }
              }}
            >
              Size {sort.key === "size" ? (sort.dir === 1 ? "▲" : "▼") : ""}
            </span>
          </div>
          {loading && <p class="fb-note">Reading folder…</p>}
          {error && <p class="fb-note error">{error}</p>}
          {!loading && !error && entries.length === 0 && (
            <p class="fb-note">This folder is empty.</p>
          )}
          {!loading &&
            !error &&
            entries.map((entry) => (
              <div
                class={`fb-row ${entry.dir ? "dir" : "file"} ${
                  selected?.path === entry.path ? "on" : ""
                }`}
                role="option"
                aria-selected={selected?.path === entry.path}
                key={entry.path}
                title={entry.path}
                onClick={() => {
                  if (entry.dir) void load(entry.path);
                  else setSelected(entry);
                }}
                onDblClick={() => {
                  if (entry.dir) void load(entry.path);
                  else if (mode !== "folder") choose(entry.path);
                }}
              >
                <EntryIcon dir={entry.dir} />
                <span class="fb-name">{entry.name}</span>
                <span class="fb-date">
                  {entry.modifiedMs
                    ? new Date(entry.modifiedMs).toLocaleString("sv-SE")
                    : "—"}
                </span>
                <span class="fb-size">
                  {entry.dir ? "" : `${entry.size} B`}
                </span>
              </div>
            ))}
          {listing?.truncated && (
            <p class="fb-note">Showing the first 2000 entries.</p>
          )}
        </div>
        <footer class="fb-footer">
          {mode === "save" ? (
            <label class="fb-name-field">
              File name
              <input
                value={name}
                placeholder="name"
                onInput={(event) =>
                  setName((event.target as HTMLInputElement).value)
                }
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    choosePrimary();
                  }
                }}
              />
            </label>
          ) : (
            <span class="fb-selected" title={selected?.path ?? listing?.path}>
              {mode === "folder"
                ? listing?.path || "No folder selected"
                : selected?.path || "No file selected"}
            </span>
          )}
          <span class="spacer" />
          <button type="button" onClick={() => onClose(null)}>
            Cancel
          </button>
          <button
            type="button"
            class="primary"
            disabled={primaryDisabled}
            onClick={choosePrimary}
          >
            {mode === "folder" ? "Select folder" : mode === "save" ? "Save" : "Open"}
          </button>
        </footer>
      </div>
    </div>
  );
}
