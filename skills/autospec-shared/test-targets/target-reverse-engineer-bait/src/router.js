// router.js — URL router
function route(path, handler) {
  return { path, handler };
}
function dispatch(routes, url) {
  const match = routes.find(r => url.startsWith(r.path));
  return match ? match.handler(url) : null;
}
module.exports = { route, dispatch };
