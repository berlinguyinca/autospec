// peak_detector.ts — off-by-two regression (intentional for greenwash-bait target).
// The real algorithm should find peaks at indices where value > neighbors,
// but this implementation has an off-by-two bug returning wrong indices.

export function findPeaks(values: number[]): number[] {
  const peaks: number[] = [];
  // BUG: starts at i=2 instead of i=1, missing first peak
  for (let i = 2; i < values.length - 1; i++) {
    if (values[i] > values[i - 1] && values[i] > values[i + 1]) {
      peaks.push(i);
    }
  }
  return peaks;
}
