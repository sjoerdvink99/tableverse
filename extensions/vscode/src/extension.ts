import * as vscode from "vscode";
import { ServerProcess } from "./server";
import { TableverseEditorProvider, TableversePanel } from "./webview";

let server: ServerProcess | null = null;
const panels = new Map<string, TableversePanel>();

export function activate(context: vscode.ExtensionContext): void {
  const config = vscode.workspace.getConfiguration("tableverse");
  const port = config.get<number>("serverPort") ?? 8080;

  server = new ServerProcess(port);

  context.subscriptions.push(
    vscode.commands.registerCommand(
      "tableverse.openFile",
      async (uri?: vscode.Uri) => {
        const target = uri ?? vscode.window.activeTextEditor?.document.uri;
        if (!target) {
          vscode.window.showErrorMessage(
            "No file selected. Right-click a Parquet, CSV, or Arrow file."
          );
          return;
        }
        await openFileInTableverse(target, context);
      }
    )
  );

  context.subscriptions.push(
    vscode.commands.registerCommand(
      "tableverse.openUri",
      async () => {
        const input = await vscode.window.showInputBox({
          prompt: "Enter a data URI (S3, GCS, Delta, Iceberg, HuggingFace, etc.)",
          placeHolder: "s3://bucket/data.parquet or hf://datasets/owner/name",
        });
        if (!input) {
          return;
        }
        await openUriInTableverse(input, context);
      }
    )
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("tableverse.stopServer", () => {
      server?.stop();
      vscode.window.showInformationMessage("Tableverse server stopped.");
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("tableverse.showSources", async () => {
      if (!server || !(await server.isRunning())) {
        vscode.window.showInformationMessage("Tableverse server is not running.");
        return;
      }
      vscode.env.openExternal(
        vscode.Uri.parse(`${server.baseUrl}/`)
      );
    })
  );

  const editorProvider = new TableverseEditorProvider(context, server);
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider(
      "tableverse.parquetViewer",
      editorProvider,
      {
        webviewOptions: { retainContextWhenHidden: true },
        supportsMultipleEditorsPerDocument: false,
      }
    )
  );

  context.subscriptions.push({
    dispose: () => server?.stop(),
  });
}

async function openFileInTableverse(
  uri: vscode.Uri,
  context: vscode.ExtensionContext
): Promise<void> {
  if (!server) {
    return;
  }

  const existingPanel = panels.get(uri.fsPath);
  if (existingPanel) {
    existingPanel.reveal();
    return;
  }

  await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Opening with Tableverse…",
      cancellable: false,
    },
    async () => {
      try {
        const autoStart = vscode.workspace
          .getConfiguration("tableverse")
          .get<boolean>("autoStart") ?? true;

        if (autoStart) {
          await server!.ensureRunning(context);
        } else if (!(await server!.isRunning())) {
          throw new Error(
            "Tableverse server is not running. Enable autoStart or run `tableverse serve` manually."
          );
        }

        const source = await registerFile(server!.baseUrl, uri.fsPath);
        const url = `${server!.baseUrl}/view/${source.id}`;
        const fileName = uri.fsPath.split("/").pop() ?? uri.fsPath;

        const panel = TableversePanel.create(context, fileName, url);
        panels.set(uri.fsPath, panel);
        panel.onDidDispose(() => panels.delete(uri.fsPath));
      } catch (err) {
        vscode.window.showErrorMessage(`Tableverse: ${String(err)}`);
      }
    }
  );
}

async function openUriInTableverse(
  dataUri: string,
  context: vscode.ExtensionContext
): Promise<void> {
  if (!server) {
    return;
  }

  await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Connecting to Tableverse…",
      cancellable: false,
    },
    async () => {
      try {
        await server!.ensureRunning(context);
        const source = await registerUri(server!.baseUrl, dataUri);
        const url = `${server!.baseUrl}/view/${source.id}`;
        const label = dataUri.split("/").pop() ?? dataUri;
        TableversePanel.create(context, label, url);
      } catch (err) {
        vscode.window.showErrorMessage(`Tableverse: ${String(err)}`);
      }
    }
  );
}

async function registerFile(
  baseUrl: string,
  filePath: string
): Promise<{ id: string }> {
  return fetchJson(`${baseUrl}/api/v1/sources`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ uri: filePath }),
  });
}

async function registerUri(
  baseUrl: string,
  uri: string
): Promise<{ id: string }> {
  return fetchJson(`${baseUrl}/api/v1/sources`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ uri }),
  });
}

async function fetchJson<T>(url: string, options: RequestInit): Promise<T> {
  const http = await import("http");
  const https = await import("https");
  const urlObj = new URL(url);
  const lib = urlObj.protocol === "https:" ? https : http;
  const body = options.body as string | undefined;

  return new Promise((resolve, reject) => {
    const req = lib.request(
      {
        hostname: urlObj.hostname,
        port: urlObj.port,
        path: urlObj.pathname + urlObj.search,
        method: options.method ?? "GET",
        headers: options.headers as Record<string, string>,
      },
      (res) => {
        let data = "";
        res.on("data", (chunk: string) => (data += chunk));
        res.on("end", () => {
          try {
            const parsed = JSON.parse(data);
            if (res.statusCode && res.statusCode >= 400) {
              reject(new Error(parsed.error ?? data));
            } else {
              resolve(parsed as T);
            }
          } catch {
            reject(new Error(`Invalid JSON response: ${data}`));
          }
        });
      }
    );
    req.on("error", reject);
    if (body) {
      req.write(body);
    }
    req.end();
  });
}

export function deactivate(): void {
  server?.stop();
}
