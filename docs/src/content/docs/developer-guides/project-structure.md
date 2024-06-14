---
title: Project Structure
description: This is a page in my Starlight-powered site
---

## Core

### Data Source and Structure

Zhang's design goal is to be format-agnostic, allowing you to store your data in databases, text files, binary files, or
anywhere that offers flexibility.
To ensure seamless functionality, we've abstracted two components to handle diverse data sources and structures.

#### Data Source

A Data Source refers to the origin of your data, whether it's a local file system or a remote GitHub repository. The
Data Source informs the Zhang core how to:

* Read data and convert it into a standardized, Zhang-compatible Directive
* Write a Directive back to the data source, typically for data updates
  When defining a Data Source, you must specify the corresponding data type (DataType).

#### Data Type

A Data Type represents the structure of the source data, such as plain text, JSON, or databases. The core function of a
DataType is to convert standardized Directives into compatible storage formats. Zhang officially supports the following
data formats:

* Zhang (improved beancount plain text format)
* Beancount (official beancount plain text format)