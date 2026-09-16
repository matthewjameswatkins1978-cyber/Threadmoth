export function loadConfig(path: string): string {
  return path.trim();
}

export function saveConfig(path: string, value: string): void {
  console.log(path, value);
}

export class Service {
  constructor(public readonly name: string) {}
}
