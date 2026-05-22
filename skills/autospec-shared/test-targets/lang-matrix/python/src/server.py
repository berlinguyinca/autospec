"""server.py — FastAPI HTTP server."""
from fastapi import FastAPI
from .lib import greet

app = FastAPI()


@app.get("/greet/{name}")
def greet_endpoint(name: str) -> dict:
    """Return a greeting for name."""
    return {"message": greet(name)}
