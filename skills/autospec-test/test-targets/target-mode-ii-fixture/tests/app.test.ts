// target-mode-ii-fixture unit tests — in-scope only.
import { getFamilyById, updateFamily, Family } from '../src/app';

const testDb: Family[] = [
  { id: 'test-family-fixture-01', name: 'Test Family' },
  { id: 'other-family-999', name: 'Other Family' },
];

describe('getFamilyById', () => {
  it('returns family for in-scope id', () => {
    const result = getFamilyById(testDb, 'test-family-fixture-01');
    expect(result?.id).toBe('test-family-fixture-01');
  });

  it('returns undefined for unknown id', () => {
    expect(getFamilyById(testDb, 'nonexistent')).toBeUndefined();
  });
});
