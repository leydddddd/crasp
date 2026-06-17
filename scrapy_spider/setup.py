from setuptools import setup, find_packages

setup(
    name="crasp-spider",
    version="0.1.0",
    packages=find_packages(),
    install_requires=[
        "scrapy>=2.11",
        "parsel>=1.8",
    ],
)
