# /// script
# requires-python = ">=3.9"
# dependencies = [
#     "pillow",
#     "numpy",
# ]
# ///


from PIL import Image
import numpy as np
import sys


def binary_file_to_grayscale_image(input_file, output_image):
    # Read the binary file
    with open(input_file, "rb") as file:
        data = file.read(1000000)

    # Ensure the file contains exactly 1 million bytes
    if len(data) != 1000000:
        raise ValueError(f"The input file must contain exactly 1 million bytes, got {len(data)} bytes")

    # Convert the binary data to a numpy array
    pixel_values = np.frombuffer(data, dtype=np.uint8)

    # Reshape the array to 1000x1000
    pixel_values = pixel_values.reshape((1000, 1000))

    # Create an image from the numpy array
    image = Image.fromarray(pixel_values, mode="L")

    # Save the image
    image.save(output_image)
    print(f"Grayscale image saved as {output_image}")


def main() -> None:
    if len(sys.argv) != 3:
        print("Usage: db_to_img.py <input> <output>", file=sys.stderr)
        sys.exit(2)
    input = sys.argv[1]
    output = sys.argv[2]
    binary_file_to_grayscale_image(input, output)


if __name__ == "__main__":
    main()
