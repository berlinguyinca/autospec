// target-failing-gap: App with drag-handle button not covered by E2E tests.

export function greet(name: string): string {
  return `Hello, ${name}!`;
}

export function add(a: number, b: number): number {
  return a + b;
}

export function formatTitle(title: string): string {
  return title.trim().toUpperCase();
}

// This component has a drag-handle button that is intentionally NOT covered
// by E2E tests, to trigger the missing_ui_elements gate failure.
export function DragHandle() {
  return '<button data-testid="drag-handle">Drag</button>';
}
