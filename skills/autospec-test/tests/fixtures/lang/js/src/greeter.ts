// Greeter module — TypeScript fixture for function-presence tests

export function greet(name: string): string {
  return `Hello, ${name}!`;
}

export function farewell(name: string): string {
  return `Goodbye, ${name}!`;
}

// Arrow function export
export const formatName = (first: string, last: string): string =>
  `${first} ${last}`;

// Private (not exported) — should NOT appear in exported list
function _internal(): void {
  console.log('internal');
}
