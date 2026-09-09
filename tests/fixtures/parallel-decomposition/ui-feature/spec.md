# Spec: demo-ui terminal UI feature

A UI/API/persistence feature: a terminal UI for `demo-ui` with view model,
event routing, screen state, theming, focus, layout, accessibility
annotations and session persistence.

## Capabilities

- view model consumed by the render loop
- key-event routing
- typed screen state store
- theme tokens for the TUI palette
- focus order policy
- widget layout metrics
- accessibility annotations
- JSON session persistence

## Hard dependency notes

The render pipeline consumes the view model; session persistence consumes
the event router and screen store; the theme and persistence tests verify
artifacts of the theme tokens and persist codec. Everything else is
independent.
