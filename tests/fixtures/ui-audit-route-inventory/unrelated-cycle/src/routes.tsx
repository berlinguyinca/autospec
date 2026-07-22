import { Routes, Route } from "react-router-dom";

const alpha = [{ path: "alpha", children: beta }];
const beta = [{ path: "beta", children: alpha }];

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/alive" element={<div>{alpha.length + beta.length}</div>} />
    </Routes>
  );
}
