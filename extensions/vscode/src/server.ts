import * as cp from "child_process";
import * as http from "http";
import * as path from "path";
import * as vscode from "vscode";

const HEALTH_TIMEOUT_MS = 15_000;
const HEALTH_POLL_MS = 200;

export class ServerProcess {
  private process: cp.ChildProcess | null = null;
  private _port: number;
  private _baseUrl: string;

  constructor(port: number) {
    this._port = port;
    this._baseUrl = `http://localhost:${port}`;
  }

  get port(): number {
    return this._port;
  }

  get baseUrl(): string {
    return this._baseUrl;
  }

  async ensureRunning(context: vscode.ExtensionContext): Promise<void> {
    if (await this.isRunning()) {
      return;
    }
    await this.start(context);
  }

  async isRunning(): Promise<boolean> {
    return new Promise((resolve) => {
      const req = http.get(`${this._baseUrl}/healthz`, (res) => {
        resolve(res.statusCode === 200);
      });
      req.on("error", () => resolve(false));
      req.setTimeout(1000, () => {
        req.destroy();
        resolve(false);
      });
    });
  }

  private async start(context: vscode.ExtensionContext): Promise<void> {
    const binary = this.findBinary(context);
    if (!binary) {
      throw new Error(
        "tableverse binary not found. " +
          "Install via cargo: cargo install tableverse, " +
          "or set tableverse.binaryPath in settings."
      );
    }

    this.process = cp.spawn(binary, ["serve", "--port", String(this._port), "--no-open"], {
      stdio: "ignore",
      detached: false,
    });

    this.process.on("error", (err) => {
      vscode.window.showErrorMessage(`Tableverse server error: ${err.message}`);
    });

    await this.waitReady();
  }

  private findBinary(context: vscode.ExtensionContext): string | null {
    const config = vscode.workspace.getConfiguration("tableverse");
    const configuredPath = config.get<string>("binaryPath");

    if (configuredPath && configuredPath.length > 0) {
      return configuredPath;
    }

    const extensionBin = path.join(context.extensionPath, "bin", "tableverse");
    try {
      require("fs").accessSync(extensionBin);
      return extensionBin;
    } catch {
      /* not found */
    }

    const pathDirs = (process.env.PATH || "").split(path.delimiter);
    for (const dir of pathDirs) {
      const candidate = path.join(dir, "tableverse");
      try {
        require("fs").accessSync(candidate);
        return candidate;
      } catch {
        /* not found */
      }
    }

    return null;
  }

  private waitReady(): Promise<void> {
    return new Promise((resolve, reject) => {
      const deadline = Date.now() + HEALTH_TIMEOUT_MS;
      const poll = () => {
        if (Date.now() > deadline) {
          reject(
            new Error(
              `Tableverse server did not start within ${HEALTH_TIMEOUT_MS / 1000}s`
            )
          );
          return;
        }
        this.isRunning().then((running) => {
          if (running) {
            resolve();
          } else {
            setTimeout(poll, HEALTH_POLL_MS);
          }
        });
      };
      poll();
    });
  }

  stop(): void {
    if (this.process) {
      this.process.kill();
      this.process = null;
    }
  }
}
