# LUD - CLI Tool for Easy File Transfers

LUD is a lightweight command-line tool that simplifies file transfers by allowing you to easily upload, download, and share files with a simple interface.

## Installation

To install LUD, run the following command:

```bash
cargo install lud
```

## Usage

### Start a Server

To start the file transfer server and specify the storage directory:

```bash
lud ln -o ./storage
```

This will initiate a server on the default port and store files in the `./storage` directory.

### Upload a File

To upload a file to the server:

```bash
lud u example.txt
```

This command uploads `example.txt` to the server for others to access.

### Download a File

To download a file from the server:

```bash
lud d example.txt
```

This will fetch the `example.txt` file from the server to your local machine.

### Additional Help

For more options and usage details, you can run:

```bash
lud --help
```

This will display a list of available commands and options.
