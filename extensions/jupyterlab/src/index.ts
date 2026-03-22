import {
  JupyterFrontEnd,
  JupyterFrontEndPlugin,
} from "@jupyterlab/application";
import { IFileBrowserFactory } from "@jupyterlab/filebrowser";
import { ILauncher } from "@jupyterlab/launcher";
import { IMainMenu } from "@jupyterlab/mainmenu";
import { Widget } from "@lumino/widgets";

const TABLEVERSE_MIME = "application/x-tableverse";
const SUPPORTED_EXTENSIONS = [".parquet", ".arrow", ".csv", ".json", ".jsonl"];

let widgetCounter = 0;

function serverUrl(): string {
  const meta = document.querySelector<HTMLMetaElement>(
    'meta[name="tableverse-server-url"]'
  );
  if (meta?.content) return meta.content.replace(/\/$/, "");
  return "http://localhost:8080";
}

class TableverseWidget extends Widget {
  constructor(sourceUrl: string, title: string) {
    super();
    this.id = `tableverse-${++widgetCounter}`;
    this.title.label = title;
    this.title.closable = true;

    const iframe = document.createElement("iframe");
    iframe.src = sourceUrl;
    iframe.setAttribute("style", "width:100%;height:100%;border:none;display:block;");
    this.node.appendChild(iframe);
    this.node.style.overflow = "hidden";
  }
}

async function registerFileSource(
  filePath: string,
  baseUrl: string
): Promise<string> {
  const res = await fetch(`${baseUrl}/api/v1/sources`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ uri: filePath }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`Tableverse: failed to register source (${res.status})${text ? `: ${text}` : ""}`);
  }
  const data = await res.json() as Record<string, unknown>;
  if (typeof data["id"] !== "string") {
    throw new Error("Tableverse: server returned invalid source response");
  }
  return `${baseUrl}/view/${data["id"]}`;
}

async function uploadBytes(
  bytes: ArrayBuffer,
  name: string,
  isParquet: boolean,
  baseUrl: string
): Promise<string> {
  const res = await fetch(`${baseUrl}/api/v1/upload`, {
    method: "PUT",
    headers: {
      "Content-Type": isParquet ? "application/x-parquet" : "application/octet-stream",
      "X-TV-Name": name,
    },
    body: bytes,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`Tableverse: upload failed (${res.status})${text ? `: ${text}` : ""}`);
  }
  const data = await res.json() as Record<string, unknown>;
  if (typeof data["id"] !== "string") {
    throw new Error("Tableverse: server returned invalid upload response");
  }
  return `${baseUrl}/view/${data["id"]}`;
}

function openWidget(
  app: JupyterFrontEnd,
  viewUrl: string,
  label: string
): void {
  const widget = new TableverseWidget(viewUrl, label);
  app.shell.add(widget, "main");
  app.shell.activateById(widget.id);
}

const extension: JupyterFrontEndPlugin<void> = {
  id: "@tableverse/jupyterlab-extension:plugin",
  autoStart: true,
  optional: [IFileBrowserFactory, ILauncher, IMainMenu],
  activate: (
    app: JupyterFrontEnd,
    _browserFactory: IFileBrowserFactory | null,
    _launcher: ILauncher | null,
    mainMenu: IMainMenu | null
  ) => {
    const base = serverUrl();

    app.commands.addCommand("tableverse:open-file", {
      label: "Open in Tableverse",
      execute: async (args) => {
        const filePath = typeof args["path"] === "string" ? args["path"] : undefined;
        if (!filePath) return;
        try {
          const viewUrl = await registerFileSource(filePath, base);
          const name = filePath.split("/").pop() ?? filePath;
          openWidget(app, viewUrl, name);
        } catch (err) {
          console.error("[Tableverse]", err);
        }
      },
    });

    app.commands.addCommand("tableverse:render-mime", {
      label: "Render Tableverse output",
      execute: async (args) => {
        const sourceUrl = typeof args["url"] === "string" ? args["url"] : undefined;
        const label = typeof args["label"] === "string" ? args["label"] : "Tableverse";
        if (!sourceUrl) return;
        openWidget(app, sourceUrl, label);
      },
    });

    app.contextMenu.addItem({
      command: "tableverse:open-file",
      selector: SUPPORTED_EXTENSIONS.map(
        (ext) => `[data-file-type="${ext.slice(1)}"]`
      ).join(", "),
      rank: 10,
    });

    if (mainMenu) {
      mainMenu.fileMenu.addGroup(
        [{ command: "tableverse:open-file" }],
        40
      );
    }

    app.docRegistry.addFileType({
      name: "parquet",
      displayName: "Parquet file",
      extensions: [".parquet"],
      mimeTypes: ["application/x-parquet"],
      iconClass: "jp-MaterialIcon jp-SpreadsheetIcon",
    });

    app.docRegistry.addFileType({
      name: "arrow",
      displayName: "Arrow IPC file",
      extensions: [".arrow"],
      mimeTypes: ["application/vnd.apache.arrow.file"],
      iconClass: "jp-MaterialIcon jp-SpreadsheetIcon",
    });

    registerMimeRenderer(app, base);
  },
};

interface MimeModel {
  data: Record<string, unknown>;
}

interface MimeRendererFactory {
  mimeTypes: string[];
  safe: boolean;
  render: (model: MimeModel) => Promise<Widget>;
}

interface AppWithRendermime extends JupyterFrontEnd {
  rendermime?: {
    addFactory: (factory: MimeRendererFactory) => void;
  };
}

function registerMimeRenderer(app: JupyterFrontEnd, base: string): void {
  const renderer: MimeRendererFactory = {
    mimeTypes: [TABLEVERSE_MIME],
    safe: true,
    render: async (model: MimeModel): Promise<Widget> => {
      const payload = model.data[TABLEVERSE_MIME];

      if (typeof payload === "string") {
        const viewUrl = payload.startsWith("http")
          ? payload
          : `${base}/view/${payload}`;
        return new TableverseWidget(viewUrl, "Tableverse");
      }

      if (
        payload !== null &&
        typeof payload === "object" &&
        !Array.isArray(payload)
      ) {
        const p = payload as Record<string, unknown>;
        if (
          typeof p["name"] !== "string" ||
          typeof p["format"] !== "string" ||
          !Array.isArray(p["bytes"])
        ) {
          console.error("[Tableverse] invalid MIME payload shape", p);
          return new Widget();
        }
        const buf = new Uint8Array(p["bytes"] as number[]).buffer;
        const viewUrl = await uploadBytes(
          buf,
          p["name"],
          p["format"] === "parquet",
          base
        );
        return new TableverseWidget(viewUrl, p["name"]);
      }

      console.error("[Tableverse] unrecognized MIME payload", payload);
      return new Widget();
    },
  };

  const appWithRendermime = app as unknown as AppWithRendermime;
  if (typeof appWithRendermime.rendermime?.addFactory === "function") {
    appWithRendermime.rendermime.addFactory(renderer);
  }
}

export default extension;
