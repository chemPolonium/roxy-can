/// True when the typed path is a bare file name. The host resolves those
/// into its web download folder, where /api/download can serve them.
export function isDownloadName(path: string): boolean {
  const name = path.trim();
  return name.length > 0 && !/[\\/]/.test(name);
}

/// Drops a previous file of the same name so polling cannot pick up stale
/// content. Call before the command that writes it.
export async function clearDownload(name: string): Promise<void> {
  try {
    await fetch(`/api/download?name=${encodeURIComponent(name.trim())}`, {
      method: "DELETE",
      cache: "no-store",
    });
  } catch {
    // The folder may not exist yet; the writer will create it.
  }
}

/// The export/save command runs on the core thread; poll until the host
/// reports the file instead of guessing a fixed delay.
export async function waitForDownload(
  name: string,
  attempts = 40,
): Promise<string | null> {
  const url = `/api/download?name=${encodeURIComponent(name.trim())}`;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 150));
    try {
      const response = await fetch(url, { method: "HEAD", cache: "no-store" });
      if (response.ok) return url;
    } catch {
      // The file is not there yet; keep waiting.
    }
  }
  return null;
}

export function triggerDownload(url: string, name: string): void {
  const link = document.createElement("a");
  link.href = url;
  link.download = name.trim();
  link.click();
}
