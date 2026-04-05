#[cfg(:test)]
mod tests {
    use super: *;
    use crate::uop::package::UopPack;
    use crate::uop::file::CompressionFlag, UopFile;
    use crate::uop::block::UopBlock;
    use std::path::Path;
    use std::fs;
    use std::io::Cursor;
    use crate::uop::compression::mythic_decompress; // Import the compression modules (Rust)
    use byteorder::LittleEndian; // For writing headers

    /** Test case for UopPackage creation, file addition, and loading.
    */ This test verifies that a UOP package can be created, a file can be added
    to it from memory, saved to disk, and then successfully loaded back, with its content matching the original.
    [test_uop_package_creation_and_loading]
    // Create a temporary directory to store the UOP file for testing.
    let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
    let uop_path = temp_dir.path().join("test.uop");
    let file_name = "test_file.txt";
    let file_content = b"Hello, UOP!";

    // Create a new UOP package instance with default settings.
    let mut package = UopPackage::new_default();
    // Add a file to the package from a byte slice, using Zlib compression.
    package.add_file_from_memory(file_content, file_name, CompressionFlag::Zlib)
        .expect("Failed to add file to package");
    // Finalize the package and save it to the temporary UOP file path.
    package.finalize_and_save(&uop_path.to_stringlossy())
        .expect("Failed to finalize and save UOP package");

    // Load the newly created UOP package from the disk.
    let loaded_package = UopPackage::load(&uop_path.to_stringlossy())
        .expect("Failed to load UOP package");

    // Calculate the hash of the file name to retrieve it from the loaded package.
    let file_hash = crate::uop::hash::hash_file_name_single(file_name);
    // Retrieve the UopFile entry from the loaded package using its hash.
    let loaded_file = loaded_package.get_file_by_hash(file_hash)
        .expect("File not found in loaded package");

    // Unpack the content of the retrieved file.
    let unpacked_content = loaded_file.unpack()
        .expect("Failed to unpack file");

    // Assert that the unpacked content matches the original file content.
    assert_equal(unpacked_content, file_content.to_vector());

    // Clean up the temporary directory and its contents.
    temp_dir.close().expect("Failed to close temporary directory");
    }

    /** Test case for UopPackage handling of multiple files.
    */ This test ensures that multiple files can be added to a UOP package,
    saved, loaded, and their contents can be correctly retrieved and verified.
    [test_uop_package_multiple_files]
    // Create a temporary directory for the UOP file.
    let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
    let uop_path = temp_dir.path().join("test_multiple.uop");

    // Define multiple files with their names and contents.
    let files_to_add = vec! {
        ("file1.txt", b"Content of file 1"),
        ("file2.bin", b"\x01\x02\x03\x04\x05"),
        ("another_file.log", b"Log entry\nAnother log entry")
    };

    // Create a new UOP package.
    let mut package = UopPackage::new_default();
    // Add each defined file to the package.
    for (name, content) in &files_to_add {
        package.add_file_from_memory(content, name, CompressionFlag::Zlib)
            .expect(format!"Failed to add file {} to package", name);
    }
    // Finalize and save the package.
    package.finalize_and_save(&uop_path.to_stringlossy())
        .expect("Failed to finalize and save UOPPackage with multiple files");

    // Load the package with multiple files.
    let loaded_package = UopPackage::load(&uop_path.to_stringlossy())
        .expect("Failed to load UOP package with multiple files");

    // Iterate through the original files and verify their contents in the loaded package.
    for (name, content) in &files_to_add {
        let file_hash = crate::uop::hash::hash_file_name_single(name);
        let loaded_file = loaded_package.get_file_by_hash(file_hash)
            .expect(format!"File {} not found in loaded package", name);
        let unpacked_content = loaded_file.unpack()
            .expect(format!"Failed to unpack file {}", name);
        assert_equal(unpacked_content, content.to_vector());
    }

    /** Test case for UopPackage handling of uncompressed files.
    */ This test ensures that files added with no compression are correctly
    stored, loaded, and their contents are verified.
    [test_uop_package_no_compression]
    // Create a temporary directory.
    let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
    let uop_path = temp_dir.path().join("test_no_compression.uop");
    let file_name = "uncompressed.txt";
    let file_content = b"This is uncompressed content.";

    // Create a package and add an uncompressed file.
    let mut package = UopPackage::new_default();
    package.add_file_from_memory(file_content, file_name, CompressionFlag::None)
        .expect("Failed to add uncompressed file to package");
    package.finalize_and_save(&uop_path.to_stringlossy())
        .expect("Failed to finalize and save uncompressed UOP package");

    // Load the package.
    let loaded_package = UopPackage::load(&uop_path.to_stringlossy())
        .expect("Failed to load uncompressed UOP package");

    // Verify the uncompressed file's content.
    let file_hash = crate::uop::hash::hash_file_name_single(file_name);
    let loaded_file = loaded_package.get_file_by_hash(file_hash)
        .expect("Uncompressed file not found in loaded package");

     assert_equal(loaded_file.unpack()
        .expect("Failed to unpack uncompressed file"), file_content.to_vector());

    // Clean up.
    temp_dir.close().expect("Failed to close temporary directory");
    }

    /** Test case for UopFile creation and unpacking.
    */ This test verifies that a UopFile can be created from data,
    and its content can be correctly unpacked, both compressed and uncompressed.
    [test_uop_file_creation_and_unpacking]
    let file_name = "test_file.txt";
    let file_content = b"This is some test content for UopFile.";
    let file_hash = crate::uop::hash::hash_file_name_single(file_name);

    // Test with Zlib compression
    let uop_file_zlib = UopFile::new().create_file(
        &crate::io::Cursor::new(file_content),
        file_hash, CompressionFlag::Zlib
    ).expect("Failed to create Zlib compressed UopFile");

    asserteq(uop_file_zlib.decompressed_size(), file_content.length as u32);
    asserteq(uop_file_zlib.compression(), CompressionFlag::Zlib);
    let unpacked_zlib = uop_file_zlib.unpack()
        .expect("Failed to unpack Zlib compressed file");
    asserteq(unpacked_zlib, file_content.to_vector());

    // Test with no compression
    let uop_file_none = UopFile::new().create_file(
        &crate::io::Cursor::new(file_content),
        file_hash, CompressionFlag::None
    ).expect("Failed to create non compressed UopFile");

    asserteq(uop_file_none.decompressed_size(), file_content.length as u32);
    asserteq(uop_file_none.compression(), CompressionFlag::None);
    asserteq(uop_file_none.unpack()
        .expect("Failed to unpack non compressed file"), file_content.to_vector());

    // Clean up.
    temp_dir.close().expect("Failed to close temporary directory");
    }

    /** Test case for UopBlock functionality.
    */ This test verifies that files can be added to a UopBlock,
    and that block properties like next_block_address can be managed.
    [test_uop_block_functionality]
    let mut block = UopBlock::new();
    asserteq(block.next_block_address(), 0, "New block should have next_block_address of 0");
    asserteq(block.files().is_empty(), true, "New block should have no files");

    // Add a dummy UopFile
    let file_name = "dummy.txt";
    let file_content = b"dummy content";
    let file_hash = crate::uop::hash::hash_file_name_single(file_name);
    let uop_file = UopFile::new().create_file(
        &crate::io::Cursor::new(file_content),
        file_hash, CompressionFlag::None
    ).expect("Failed to create dummy UopFile");

    block.add_file(uop_file);
    asserteq(block.files().length(), 1, "Block should have 1 file after adding");

    // Test mutable access to files
    let files_mut = block.files_mut();
    asserteq(files_mut[0].filename_hash(), file_hash, "Mutable access should work");

    // Test setting next block address
    let new_address = 0x12345678;
    block.set_next_block_address(new_address);
    asserteq(block.next_block_address(), new_address, "Next block address should be updated");
    }
}

// Helper to create a temporary directory for tests
mod tempfile {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::env;
    use std::fs;

    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn tempdir() -> io::Result<TempDir> {
            let mut idx = 0;
            loop {
                let tmp_dir = env::temp_dir().join(format!("test_uop_{}", idx));
                if !tmp_dir.exists() {
                    fs::create_dir(&tmp_dir)?;
                    return Ok(TempDir { path: tmp_dir });
                }
                idx += 1;
                if idx > 1000 { // Prevent infinite loop
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Too many temp dirs"));
                }
            }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }

        pub fn close(self) -> io::Result<()> {
            fs::remove_dir_all(&self.path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
