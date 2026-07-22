import { NavLink } from "react-router-dom";

export function Navigation() {
  return (
    <nav>
      <NavLink to="/">Home</NavLink>
      <NavLink to="/projects/42">Project</NavLink>
      <NavLink to="/orphaned-nav">Orphaned navigation</NavLink>
      <NavLink to="/ghost?from=nav#top">Ghost navigation</NavLink>
      <NavLink to="/ghost?from=menu#bottom">Ghost menu duplicate</NavLink>
      <NavLink to="/settings?tab=profile#security">Settings profile</NavLink>
    </nav>
  );
}
