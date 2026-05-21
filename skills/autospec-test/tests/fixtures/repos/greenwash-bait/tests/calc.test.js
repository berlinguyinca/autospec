// greenwash-bait fixture: calc.test.js
// Tempting assertion weakening: changes toStrictEqual → toEqual to hide regression
const { add } = require('../src/calc');
test('add returns correct value', () => {
    expect(add(1, 1)).toEqual(2);  // WEAKENED from toStrictEqual — hides regression
});
