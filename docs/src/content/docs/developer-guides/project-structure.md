---
title: Project Structure
description: An in-depth guide to the structure of the Zhang project and best practices for extending it.
---

## Introduction

The Zhang project is designed with modularity and flexibility in mind, allowing for easy customization and extension. This document outlines the core components of the project structure, their purposes, and how they interact with each other.

## Core Components

### Data Source and Structure

Zhang's architecture is built to be agnostic of data formats, enabling data storage in various forms such as databases, text files, binary files, or any medium that offers flexibility. To facilitate this, we've abstracted two key components to manage diverse data sources and structures efficiently.

#### Data Source

The Data Source component specifies where your data is stored, whether it's on a local file system, a remote GitHub repository, or elsewhere. It is responsible for:

* Reading data and transforming it into standardized, Zhang-compatible Directives.
* Writing Directives back to the data source for updates.
  
When configuring a Data Source, it's crucial to specify the associated data type (DataType) to ensure proper handling and conversion.

#### Data Type

The Data Type component defines the structure of the source data, such as plain text, JSON, databases, etc. Its primary function is to convert standardized Directives into the storage format compatible with the chosen Data Source. Zhang officially supports several data formats, including:

* Zhang: An enhanced Beancount plain text format.
* Beancount: The official Beancount plain text format.

## Extending the Project

### Best Practices

When extending the Zhang project, consider the following best practices to maintain the integrity and modularity of the system:

* **Modular Design**: Keep extensions modular to facilitate easy updates and maintenance. Avoid tightly coupling new features with core components.
* **Data Source Compatibility**: Ensure that extensions are compatible with existing Data Sources or provide clear documentation on integrating new Data Sources.
* **Testing**: Rigorously test new components and extensions to ensure they work seamlessly with the existing system and do not introduce regressions.
* **Documentation**: Document any new features or changes thoroughly to assist future developers in understanding and utilizing your extensions.

## Conclusion

Understanding the Zhang project structure and adhering to best practices for extension will ensure that enhancements are both effective and maintainable. This approach allows for the seamless integration of new features, ensuring the project remains robust and adaptable to future requirements.
