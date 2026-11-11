# Contributing Guide

This document describes contribution guidelines for the regroup project.

## Language

All code, commits, and documentation must be in English. This is an English-language repository.

## Backward Compatibility

You can ask me about retro-compatibility, but do not decide on your own.

The project follows a **no backward compatibility** policy:
- Formats, APIs, and behavior may change between versions
- Do not add compatibility shims
- Do not add deprecated aliases
- Do not implement dual code paths
- Old on-disk data will not be supported

See the [Development Guide](development.md) for more details on coding rules.
