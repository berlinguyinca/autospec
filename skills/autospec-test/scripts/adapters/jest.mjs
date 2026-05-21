// adapters/jest.mjs
// Assertion-shift adapter for Jest test files.
// Export: bucket(fileDiff, filePath) -> Verdict[]

import { bucket as genericBucket } from './generic.mjs';

export function bucket(fileDiff, filePath) {
    return genericBucket(fileDiff, filePath);
}
