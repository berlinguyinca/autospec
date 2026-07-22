import { createBrowserRouter } from "react-router-dom";

const recursiveRoutes = [
  { path: "loop", children: recursiveRoutes },
];

export default createBrowserRouter(recursiveRoutes);
