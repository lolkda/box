interface SpawnChild {
  stdout: { emit: (event: string, data: string) => void };
  stderr: { emit: (event: string, data: string) => void };
  emit: (event: string, ...payload: unknown[]) => void;
}

function unwrapShellCommand(value: string): string {
  let command = value;
  if (command.startsWith("'") && command.endsWith("'")) command = command.slice(1, -1);
  return command.replace(/'\\''/g, "'");
}

export interface MockCommandResult {
  errno?: number;
  stdout: string;
  stderr?: string;
}

export function installKsuSpawnMock(respond: (command: string) => MockCommandResult): void {
  const bridge = {
    spawn(_command: string, args: string, _options: string, callbackName: string) {
      const shellArgs = JSON.parse(args) as string[];
      const result = respond(unwrapShellCommand(shellArgs[1] || ""));
      window.setTimeout(() => {
        const child = (window as unknown as Record<string, SpawnChild>)[callbackName];
        if (!child) return;
        if (result.stdout) {
          for (const line of result.stdout.split("\n")) child.stdout.emit("data", line);
        }
        if (result.stderr) {
          for (const line of result.stderr.split("\n")) child.stderr.emit("data", line);
        }
        child.emit("exit", result.errno ?? 0);
      }, 5);
    },
  };
  (window as Window & { ksu?: unknown }).ksu = bridge;
  (globalThis as unknown as { ksu?: unknown }).ksu = bridge;
}

export function clearKsuSpawnMock(): void {
  delete (window as Window & { ksu?: unknown }).ksu;
  delete (globalThis as unknown as { ksu?: unknown }).ksu;
}
