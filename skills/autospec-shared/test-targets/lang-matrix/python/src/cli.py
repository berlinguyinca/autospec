#!/usr/bin/env python3
"""cli.py — CLI entry point using click."""
import click
from .lib import greet


@click.command()
@click.argument("name")
def main(name: str) -> None:
    """Greet NAME."""
    click.echo(greet(name))


if __name__ == "__main__":
    main()
