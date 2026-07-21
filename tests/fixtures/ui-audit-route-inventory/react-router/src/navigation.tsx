import { NavLink } from "react-router-dom";

export function Navigation() {
  return (
    <nav>
      <NavLink to="/">Home</NavLink>
      <NavLink to="/projects/42">Project</NavLink>
      <NavLink to="/orphaned-nav">Orphaned navigation</NavLink>
    </nav>
  );
}
