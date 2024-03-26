echo "Releasing version: $1";
trimmed_version=$(echo $1 | tail -c+2);
echo "Trimmed version: $trimmed_version";
echo "Update command: s/version = \"0.1.1\"/version = \"$trimmed_version\"/"
sed -i "s/version = \"0.1.1\"/version = \"$trimmed_version\"/" Cargo.toml