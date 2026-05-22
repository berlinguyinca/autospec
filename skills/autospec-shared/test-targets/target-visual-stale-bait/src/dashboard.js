// dashboard.js — metrics dashboard component
function renderDashboard(metrics) {
  return `<div class="dashboard">${metrics.map(m => `<div>${m}</div>`).join('')}</div>`;
}
module.exports = { renderDashboard };
