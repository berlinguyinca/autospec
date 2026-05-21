// target-clean-pass: Minimal Vite+React app with full test coverage.

export function greet(name: string): string {
  return `Hello, ${name}!`;
}

export function add(a: number, b: number): number {
  return a + b;
}

export function formatTitle(title: string): string {
  return title.trim().toUpperCase();
}
