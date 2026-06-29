def main [package_file: path] {
    let package = open $package_file
    print $"Installing ($package.name)"
}
