import { Routes, Route } from "react-router-dom";

const alpha = [{ name: "alpha", children: beta }];
const beta = [{ name: "beta", children: alpha }];

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/alive" element={<div>{alpha.length + beta.length}</div>} />
    </Routes>
  );
}
