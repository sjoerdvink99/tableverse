import * as vscode from "vscode";

export class TableversePanel {
  private static readonly viewType = "tableverse.viewer";
  private readonly panel: vscode.WebviewPanel;
  private readonly title: string;

  static create(
    context: vscode.ExtensionContext,
    title: string,
    url: string
  ): TableversePanel {
    const panel = vscode.window.createWebviewPanel(
      TableversePanel.viewType,
      title,
      vscode.ViewColumn.One,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [],
      }
    );

    const instance = new TableversePanel(panel, title);
    instance.setUrl(url);
    return instance;
  }

  private constructor(panel: vscode.WebviewPanel, title: string) {
    this.panel = panel;
    this.title = title;
  }

  setUrl(url: string): void {
    this.panel.webview.html = this.buildHtml(url);
  }

  reveal(): void {
    this.panel.reveal(vscode.ViewColumn.One);
  }

  get onDidDispose(): vscode.Event<void> {
    return this.panel.onDidDispose;
  }

  private buildHtml(url: string): string {
    return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${escapeHtml(this.title)}</title>
  <style>
    html, body {
      margin: 0;
      padding: 0;
      width: 100%;
      height: 100vh;
      overflow: hidden;
      background: #0f172a;
    }
    iframe {
      width: 100%;
      height: 100%;
      border: none;
    }
    .loading {
      display: flex;
      align-items: center;
      justify-content: center;
      height: 100vh;
      color: #94a3b8;
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
      font-size: 14px;
      gap: 10px;
    }
  </style>
</head>
<body>
  <iframe
    id="frame"
    src="${url}"
    allow="clipboard-read; clipboard-write"
    onload="document.querySelector('.loading').style.display='none'"
  ></iframe>
  <div class="loading" id="loading-overlay">
    <span>Loading Tableverse…</span>
  </div>
</body>
</html>`;
  }
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class TableverseEditorProvider
  implements vscode.CustomReadonlyEditorProvider
{
  private readonly server: import("./server").ServerProcess;
  private readonly context: vscode.ExtensionContext;

  constructor(
    context: vscode.ExtensionContext,
    server: import("./server").ServerProcess
  ) {
    this.server = server;
    this.context = context;
  }

  async openCustomDocument(
    uri: vscode.Uri
  ): Promise<vscode.CustomDocument> {
    return { uri, dispose: () => {} };
  }

  async resolveCustomEditor(
    document: vscode.CustomDocument,
    webviewPanel: vscode.WebviewPanel
  ): Promise<void> {
    webviewPanel.webview.options = {
      enableScripts: true,
    };

    try {
      await this.server.ensureRunning(this.context);
      const source = await registerSourceWithServer(
        this.server.baseUrl,
        document.uri.fsPath
      );
      const url = `${this.server.baseUrl}/view/${source.id}`;
      webviewPanel.webview.html = buildViewerHtml(document.uri.fsPath, url);
    } catch (err) {
      webviewPanel.webview.html = buildErrorHtml(String(err));
    }
  }
}

async function registerSourceWithServer(
  baseUrl: string,
  filePath: string
): Promise<{ id: string }> {
  const http = await import("http");
  const https = await import("https");

  return new Promise((resolve, reject) => {
    const body = JSON.stringify({ uri: filePath });
    const urlObj = new URL(`${baseUrl}/api/v1/sources`);
    const lib = urlObj.protocol === "https:" ? https : http;

    const req = lib.request(
      {
        hostname: urlObj.hostname,
        port: urlObj.port,
        path: urlObj.pathname,
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        },
      },
      (res) => {
        let data = "";
        res.on("data", (chunk) => (data += chunk));
        res.on("end", () => {
          try {
            resolve(JSON.parse(data));
          } catch {
            reject(new Error(`Invalid response: ${data}`));
          }
        });
      }
    );

    req.on("error", reject);
    req.write(body);
    req.end();
  });
}

function buildViewerHtml(filePath: string, url: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${escapeHtml(filePath)}</title>
  <style>
    html, body { margin: 0; padding: 0; width: 100%; height: 100vh; overflow: hidden; }
    iframe { width: 100%; height: 100%; border: none; }
  </style>
</head>
<body>
  <iframe src="${url}" allow="clipboard-read; clipboard-write"></iframe>
</body>
</html>`;
}

function buildErrorHtml(message: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; padding: 24px; color: #dc2626; }
    pre { background: #fef2f2; padding: 12px; border-radius: 6px; overflow: auto; }
  </style>
</head>
<body>
  <h3>Failed to open with Tableverse</h3>
  <pre>${escapeHtml(message)}</pre>
</body>
</html>`;
}
