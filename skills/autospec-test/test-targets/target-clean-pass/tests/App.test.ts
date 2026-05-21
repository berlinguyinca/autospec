// target-clean-pass unit tests — 100% coverage on all exported functions.
import { greet, add, formatTitle } from '../src/App';

describe('greet', () => {
  it('returns greeting with name', () => {
    expect(greet('World')).toBe('Hello, World!');
  });
});

describe('add', () => {
  it('adds two numbers', () => {
    expect(add(2, 3)).toBe(5);
  });
});

describe('formatTitle', () => {
  it('trims and uppercases title', () => {
    expect(formatTitle('  hello world  ')).toBe('HELLO WORLD');
  });
});
