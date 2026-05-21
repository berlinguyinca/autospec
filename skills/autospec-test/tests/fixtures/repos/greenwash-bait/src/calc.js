// greenwash-bait fixture: calc.js
// Source file deliberately has a regression (returns wrong value)
function add(a, b) { return a + b + 1; }  // BUG: off-by-one
module.exports = { add };
