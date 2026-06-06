// uocf/src/uop/compression.rs

pub mod move_to_front_coding {
    fn move_to_front_element(array: &mut [u8; 256], element: u8) -> i32 {
        if array[0] == element {
            return 0;
        }

        let mut element_index = -1;
        for index in (1..array.len()).rev() {
            if array[index] == element {
                element_index = index as i32;
            }

            if element_index != -1 {
                array[index] = array[index - 1];
            }
        }

        array[0] = element;
        element_index
    }

    fn move_to_front_index(array: &mut [u8; 256], element_index: usize) {
        let element = array[element_index];
        for index in (1..=element_index).rev() {
            array[index] = array[index - 1];
        }
        array[0] = element;
    }

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut symbols = [0u8; 256];
        for (index, symbol) in symbols.iter_mut().enumerate() {
            *symbol = index as u8;
        }

        let mut output = vec![0u8; input.len()];
        for (index, &byte) in input.iter().enumerate() {
            output[index] = move_to_front_element(&mut symbols, byte) as u8;
        }
        output
    }

    pub fn decode(input: &[u8]) -> Vec<u8> {
        let mut symbols = [0u8; 256];
        for (index, symbol) in symbols.iter_mut().enumerate() {
            *symbol = index as u8;
        }

        let mut output = vec![0u8; input.len()];
        for (index, &encoded_index) in input.iter().enumerate() {
            let element_index = encoded_index as usize;
            output[index] = symbols[element_index];
            move_to_front_index(&mut symbols, element_index);
        }
        output
    }
}

pub mod mythic_decompress {
    use crate::utils::math::i32_downcast_ceil_usize;

    use super::move_to_front_coding;
    use std::io::{self, Cursor, Read, Write};
    use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

    // Helper function: Frequency
    fn frequency(input: &[i32], output: &mut [u8]) {
        let mut tmp = input.to_vec(); // Copy to mutable vector

        for i in 0..256 {
            let mut value = 0;
            let mut index = 0;

            for j in 0..256 {
                if tmp[j] > value {
                    index = j;
                    value = tmp[j];
                }
            }

            if value == 0 {
                break;
            }

            output[i] = index as u8;
            tmp[index] = 0;
        }
    }

    // Helper function: ShiftLeft
    fn shift_left(input: &mut [u8], element: usize) {
        for i in 0..element {
            input[i] = input[i + 1];
        }
    }

    // Helper function: ShiftRight
    fn shift_right(input: &mut [u8], element: usize) {
        for i in (0..=element).rev() {
            if i > 0 {
                input[i] = input[i - 1];
            }
        }
    }

    // Helper function: GetIdx
    fn get_idx(input: &[u8], val: u8, non_zero_count: usize) -> u8 {
        for i in 0..input.len().min(non_zero_count) {
            if input[i] == val {
                return i as u8;
            }
        }
        0
    }

    pub fn internal_compress(input: &[u8]) -> Vec<u8> {
        let mut symbol_table = [0u8; 256];
        let mut frequency_table = [0u8; 256];
        let mut partial_input = [0i32; 256 * 3];

        // Counting frequencies
        for &byte in input {
            partial_input[byte as usize] += 1;
        }

        frequency(&partial_input[0..256], &mut frequency_table);

        let mut non_zero_count = 0;
        for i in 0..256 {
            if partial_input[i] != 0 {
                non_zero_count += 1;
            }
        }

        let mut output = vec![0u8; input.len() + non_zero_count + 1024];

        // Populate partial_input for compression
        let mut m = 0;
        for i in 0..non_zero_count {
            let freq_index = frequency_table[i] as usize;
            partial_input[freq_index + 256] = m + 1;
            m += partial_input[freq_index];
            partial_input[freq_index + 512] = m;
        }

        // Write frequency counts to the first 1024 bytes of output
        for i in 0..256 {
            output[i * 4..(i * 4) + 4].copy_from_slice(&partial_input[i].to_le_bytes());
        }

        let mut count = input.len() as isize - 1;
        let mut added_symbols: Vec<u8> = Vec::with_capacity(256);

        while count >= 0 {
            let val = input[count as usize];

            let first_val_ref = partial_input[val as usize + 512];
            let output_address = first_val_ref + 1024;

            if !added_symbols.contains(&val) {
                shift_right(&mut symbol_table, added_symbols.len());
                symbol_table[0] = val;
                added_symbols.push(val);
                output[output_address as usize] = 0;
            } else if first_val_ref >= partial_input[val as usize + 256] {
                let idx = get_idx(&symbol_table, val, added_symbols.len());
                shift_right(&mut symbol_table, idx as usize);
                symbol_table[0] = val;
                output[output_address as usize] = idx;
            }
            partial_input[val as usize + 512] -= 1; // Decrement first_val_ref

            count -= 1;
        }

        // Final pass to populate remaining output based on frequency_table
        let mut m_final = 0usize;
        for i in 0..non_zero_count {
            let freq_index = frequency_table[i] as usize;
            output[m_final + 1024] = get_idx(&symbol_table, freq_index as u8, non_zero_count);
            m_final += i32_downcast_ceil_usize(partial_input[freq_index]);
        }

        output
    }

    pub fn internal_decompress(input: &[u8]) -> Vec<u8> {
        let mut symbol_table = [0u8; 256];
        let mut frequency_table = [0u8; 256];
        let mut partial_input = [0i32; 256 * 3];

        // Read frequency data from the first 1024 bytes of input
        for i in 0..256 {
            partial_input[i] = i32::from_le_bytes(input[i * 4..(i * 4) + 4].try_into().unwrap());
        }

        let mut sum = 0;
        for i in 0..256 {
            sum += partial_input[i];
        }

        if sum == 0 {
            return Vec::new();
        }

        for i in 0..256u32 {
            symbol_table[i as usize] = i as u8;
        }

        let mut non_zero_count = 0;
        for i in 0..256u32 {
            if partial_input[i as usize] != 0 {
                non_zero_count += 1;
            }
        }

        frequency(&partial_input[0..256], &mut frequency_table);

        let mut m: i32 = 0;
        for i in 0..non_zero_count {
            let freq = frequency_table[i];
            symbol_table[input[m as usize + 1024] as usize] = freq;

            partial_input[freq as usize + 256] = m + 1;
            m += partial_input[freq as usize];
            partial_input[freq as usize + 512] = m;
            assert!(m >= 0);
        }

        let mut output = vec![0u8; sum as usize];
        let mut count = 0;
        let mut val = symbol_table[0];

        while count < sum as usize {
            let first_val_ref = partial_input[val as usize + 256];

            output[count] = val;

            if first_val_ref < partial_input[val as usize + 512] {
                let idx = input[first_val_ref as usize + 1024];
                partial_input[val as usize + 256] += 1;

                if idx != 0 {
                    shift_left(&mut symbol_table, idx as usize);
                    symbol_table[idx as usize] = val;
                    val = symbol_table[0];
                }
            } else if non_zero_count > 0 {
                non_zero_count -= 1;
                shift_left(&mut symbol_table, non_zero_count);
                val = symbol_table[0];
            }

            count += 1;
        }

        output
    }

    pub fn transform(buffer: &[u8]) -> Vec<u8> {
        let compressed_internal = internal_compress(buffer);
        move_to_front_coding::encode(&compressed_internal)
    }

    pub fn compress_with_header(buffer: &[u8]) -> io::Result<Vec<u8>> {
        let raw_len = u32::try_from(buffer.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Mythic payload exceeds 32-bit length header",
            )
        })?;

        let transformed = transform(buffer);
        let mut encoded = Vec::with_capacity(transformed.len() + 4);
        encoded.write_u32::<LittleEndian>(raw_len ^ 0x8E2C9A3D)?;
        encoded.write_all(&transformed)?;
        Ok(encoded)
    }

    pub fn detransform(buffer: &[u8]) -> Vec<u8> {
        let decoded_mtf = move_to_front_coding::decode(buffer);
        internal_decompress(&decoded_mtf)
    }

    pub fn decompress_with_header(buffer: &[u8]) -> io::Result<Vec<u8>> {
        let mut cursor = Cursor::new(buffer);
        let header = cursor.read_u32::<LittleEndian>()?;
        let data_length = header ^ 0x8E2C9A3D;

        let mut list = Vec::new();
        cursor.read_to_end(&mut list)?;

        if list.len() < 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Mythic payload is shorter than its 1024-byte frequency table",
            ));
        }

        let decoded_mtf = move_to_front_coding::decode(&list);
        let decompressed_internal = internal_decompress(&decoded_mtf);

        if data_length as usize != decompressed_internal.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Decompressed data length mismatch with header.",
            ));
        }

        Ok(decompressed_internal)
    }
}

pub mod zlib_bwt_codec {
    use super::{move_to_front_coding, mythic_decompress};
    use flate2::read::ZlibDecoder;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::{self, Cursor};
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::{Read, Write};

    // Helper function: Frequency (adapted for u8 output)
    fn frequency(input: &[i32], output: &mut [u8]) {
        let mut tmp = input.to_vec(); // Copy to mutable vector

        for i in 0..256 {
            let mut value = 0;
            let mut index = 0;

            for j in 0..256 {
                if tmp[j] > value {
                    index = j;
                    value = tmp[j];
                }
            }

            if value == 0 {
                break;
            }

            output[i] = index as u8;
            tmp[index] = 0;
        }
    }

    // Helper function: ShiftLeft (adapted for u8 input)
    fn shift_left(input: &mut [u8], max: usize) {
        for i in 0..max {
            input[i] = input[i + 1];
        }
    }

    // Helper function: BuildTable (from BwtDecompress)
    fn build_table(table: &mut [u16], start_value: u8) {
        let mut first_byte = start_value;
        let mut second_byte = 0u8;
        for (index, _) in (0..256 * 256).enumerate() {
            let val: u16 = (first_byte as u16) + ((second_byte as u16) << 8);
            table[index] = val;

            first_byte = first_byte.wrapping_add(1);
            if first_byte == 0 {
                second_byte = second_byte.wrapping_add(1);
            }
        }
        table.sort_unstable(); // Use unstable sort for potentially better performance
    }

    // InternalDecompress (from BwtDecompress)
    fn internal_decompress(input: &[u8], len: u32) -> Vec<u8> {
        let mut symbol_table = [0u8; 256]; // Changed from char to u8
        let mut frequency_table = [0u8; 256]; // Changed from char to u8
        let mut partial_input = [0i32; 256 * 3];
        partial_input.fill(0);

        for i in 0..256 {
            symbol_table[i] = i as u8;
        }

        // The C# code uses input.Slice(0, 1024).CopyTo(MemoryMarshal.AsBytes(partialInput));
        // This means the first 1024 bytes of input are interpreted as 256 i32s.
        for i in 0..256 {
            partial_input[i] = i32::from_le_bytes(input[i * 4..(i * 4) + 4].try_into().unwrap());
        }

        let mut sum: i32 = 0;
        for i in 0..256 {
            sum += partial_input[i];
        }

        let final_len: u32 = if len == 0 { sum as u32 } else { len };

        if sum as u32 != final_len {
            return Vec::new(); // Return empty if lengths mismatch
        }

        let mut output = vec![0u8; final_len as usize];

        let mut non_zero_count = 0;
        for i in 0..256 {
            if partial_input[i] != 0 {
                non_zero_count += 1;
            }
        }

        frequency(&partial_input[0..256], &mut frequency_table);

        let mut m: i32 = 0;
        for i in 0..non_zero_count {
            let freq = frequency_table[i] as usize;
            symbol_table[input[m as usize + 1024] as usize] = freq as u8;

            partial_input[freq + 256] = m + 1;
            m += partial_input[freq];
            partial_input[freq + 512] = m;
            assert!(m >= 0);
        }

        let mut val: u8 = symbol_table[0];
        let mut count = 0;

        if final_len != 0 {
            while count < final_len as usize {
                let first_val_ref = partial_input[val as usize + 256];

                output[count] = val;

                if first_val_ref >= partial_input[val as usize + 512] {
                    if non_zero_count > 0 { // C# has nonZeroCount-- > 0
                        non_zero_count -= 1;
                        shift_left(&mut symbol_table, non_zero_count);
                        val = symbol_table[0];
                    }
                } else {
                    let idx: u8 = input[first_val_ref as usize + 1024];
                    partial_input[val as usize + 256] += 1;

                    if idx != 0 {
                        shift_left(&mut symbol_table, idx as usize);
                        symbol_table[idx as usize] = val;
                        val = symbol_table[0];
                    }
                }
                count += 1;
            }
        }

        output
    }

    fn decompress_transformed(buffer: &[u8]) -> io::Result<Vec<u8>> {
        let mut reader = Cursor::new(buffer);

        let _header = reader.read_u32::<LittleEndian>()?;
        let len = 0u32;

        let mut first_char = reader.read_u8()?;

        let mut table = [0u16; 256 * 256];
        build_table(&mut table, first_char);

        let mut list = Vec::with_capacity(buffer.len() - 4); // -4 for header
        while reader.position() < reader.get_ref().len() as u64 {
            let current_value = first_char;
            let value = table[current_value as usize];
            if current_value > 0 {
                let mut current_val_mut = current_value;
                while current_val_mut > 0 {
                    table[current_val_mut as usize] = table[(current_val_mut - 1) as usize];
                    current_val_mut -= 1;
                }
            }

            table[0] = value;

            list.push(value as u8);
            first_char = reader.read_u8()?;
        }

        let output = internal_decompress(&list, len); // C# passes len, but it's always 0

        Ok(output)
    }

    // Some clients zlib-wrap the BWT stage, while older experiments stored the
    // transformed bytes directly. Accept both so existing decode behavior stays
    // intact while writes use the zlib-wrapped form expected by client loaders.
    pub fn decompress(buffer: &[u8]) -> io::Result<Vec<u8>> {
        if let Ok(decoded) = decompress_zlib_wrapped(buffer) {
            return Ok(decoded);
        }

        decompress_transformed(buffer)
    }

    fn decompress_zlib_wrapped(buffer: &[u8]) -> io::Result<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(buffer);
        let mut transformed = Vec::new();
        decoder.read_to_end(&mut transformed)?;
        decompress_transformed(&transformed)
    }

    pub fn compress(buffer: &[u8]) -> io::Result<Vec<u8>> {
        let transformed = mythic_decompress::internal_compress(buffer);
        let mut staged = Vec::with_capacity(transformed.len() + 5);
        staged.extend_from_slice(&0u32.to_le_bytes());
        staged.extend_from_slice(&move_to_front_coding::encode(&transformed));
        staged.push(0);

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&staged)?;
        encoder.finish()
    }
}
