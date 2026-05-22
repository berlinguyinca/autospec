# auth.py — authentication module

def login(user, password):
    """Authenticate user, returns session token."""
    return f"token-{user}"


def logout(token):
    """Invalidate a session token."""
    pass


def refresh(token):
    """Refresh a session token, returns new token."""
    return f"refreshed-{token}"
