# Concepts

## Spec

A spec is the durable design artifact AutoSpec writes before implementation. It explains the user goal, constraints, acceptance criteria, files likely to change, and validation approach.

## Issue Tree

AutoSpec splits a spec into a parent issue and smaller child issues. Each child issue is meant to be independently understandable and reviewable.

## Model Fit

Issues receive labels such as `ctx:*` and `reasoning:*` so operators can route work to an appropriate model or harness. The goal is not to benchmark models; it is to keep work units honest about context and reasoning needs.

## Implementation Monitor

`/autospec-run` processes ready issues, opens PRs, runs validation, asks for review, and either merges or reports blockers depending on the configured gates.

## Closeout Report

Every implementation issue should end with a result-first report: claims, proof type, before/after, artifacts, scoped git status, and the most likely hidden failure.

## Release Gate

Release readiness combines repository validation, docs drift checks, QA proof, CI state, and explicit blocker reporting. It is evidence gathering, not a marketing badge.

## Safety Boundary

AutoSpec can automate a lot of repository work, but maintainers still own production impact, credentials, destructive operations, and policy decisions.

