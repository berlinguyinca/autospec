import { Routes, Route } from "react-router-dom";

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
      <Route path="*" element={<div>Not found</div>} />
    </Routes>
  );
}
