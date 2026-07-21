import { Routes, Route } from "react-router-dom";

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<div>Home</div>} />
      <Route path="projects" element={<div>Projects</div>}>
        <Route path=":id" element={<div>Project</div>} />
      </Route>
      <Route path="/projects/:id" element={<div>Project duplicate</div>} />
      <Route path="settings" lazy={() => import("./settings")} />
      <Route path="*" element={<div>Not found</div>} />
    </Routes>
  );
}
