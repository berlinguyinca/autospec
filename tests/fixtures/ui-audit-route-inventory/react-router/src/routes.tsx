import { Routes, Route } from "react-router-dom";

const singleQuotedExample = '<Route path="/single-string" />';
const doubleQuotedExample = "<Route path='/double-string' />";
const templateExample = `<Route path="/template-string" />`;
const windowsRoot = "C:\\\\";
// <Route path="/retired-after-even-backslashes" />

export function AppRoutes() {
  // <Route path="/retired" element={<div>Retired</div>} />
  return (
    <Routes>
      <Route path="/" element={<div>Home</div>} />
      <Route path="projects" element={<div>Projects</div>}>
        <Route path=":id" element={<div>Project</div>} />
      </Route>
      <Route path="/projects/:id" element={<div>Project duplicate</div>} />
      <Route path="settings" lazy={() => import("./settings")} />
      <Route
        path="strings"
        element={<div data-note={"literal // and /* text */"}>Strings</div>}
      />
      <Route
        path="after-lexical-noise"
        element={<div>Don't hide this route: {windowsRoot}</div>}
      />
      <Route path="*" element={<div>Not found</div>} />
    </Routes>
  );
}

void singleQuotedExample;
void doubleQuotedExample;
void templateExample;
