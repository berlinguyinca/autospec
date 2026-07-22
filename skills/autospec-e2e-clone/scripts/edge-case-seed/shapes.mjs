export function applyShapeDefaults(shapeName, row, index, now = new Date()) {
  const today = now.toISOString().slice(0, 10);
  const yesterday = new Date(now - 86400000).toISOString().slice(0, 10);
  switch (shapeName) {
    case 'task_done_today': row.done_at ??= `${today}T12:00:00Z`; break;
    case 'task_done_yesterday': row.done_at ??= `${yesterday}T12:00:00Z`; break;
    case 'task_done_2_to_6_days_ago': {
      const date = new Date(now - (2 + (index % 5)) * 86400000);
      row.done_at ??= `${date.toISOString().slice(0, 10)}T12:00:00Z`;
      break;
    }
    case 'task_done_around_midnight': row.done_at ??= `${today}T23:57:00Z`; break;
    case 'multiple_tasks_same_day': row.done_at ??= `${today}T${String(10 + index).padStart(2, '0')}:00:00Z`; break;
    case 'task_in_collapsed_foldout': row.foldout_collapsed ??= '1'; break;
    case 'last_item_in_long_list': row.list_position ??= String(51 + index); break;
    default: break;
  }
}

function resolveFaker(faker, path) {
  const value = path.split('.').reduce((current, key) => current?.[key], faker);
  return typeof value === 'function' ? value() : String(value ?? `synthetic_${path}`);
}

export async function generateSyntheticRows(shapeName, catalogEntry, count, faker) {
  const rows = [];
  for (let index = 0; index < count; index += 1) {
    const row = {};
    for (const [column, expression] of Object.entries(catalogEntry.template ?? {})) {
      row[column] = typeof expression === 'string' && expression.startsWith('faker:')
        ? (faker ? resolveFaker(faker, expression.slice(6)) : `synthetic_${column}_${index}`)
        : expression;
    }
    applyShapeDefaults(shapeName, row, index);
    row._autospec_synthetic = 'true';
    rows.push(row);
  }
  return rows;
}
